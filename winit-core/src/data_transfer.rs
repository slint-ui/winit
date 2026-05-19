use std::{
    collections::BTreeSet,
    fmt::Debug,
    hash::Hash,
    io::{self, BufRead},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

/// Identifier of a data transfer.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataTransferId(i64);

impl DataTransferId {
    /// Convert the [`DataTransferId`] into the underlying integer.
    ///
    /// This is useful if you need to pass the ID across an FFI boundary, or store it in an atomic.
    pub const fn into_raw(self) -> i64 {
        self.0
    }

    /// Construct a [`DataTransferId`] from the underlying integer.
    ///
    /// This should only be called with integers returned from [`DataTransferId::into_raw`].
    pub const fn from_raw(id: i64) -> Self {
        Self(id)
    }
}

enum DataTransferDataInner {
    Paths(Vec<PathBuf>),
    Plaintext(String),
    Bytes(Box<dyn BufRead + Send + Sync>),
}

pub struct DataTransferData {
    inner: DataTransferDataInner,
}

impl DataTransferData {
    pub fn from_bytes<R>(reader: R) -> Self
    where
        R: BufRead + Send + Sync + 'static,
    {
        Self { inner: DataTransferDataInner::Bytes(Box::new(reader)) }
    }
}

impl From<Vec<PathBuf>> for DataTransferData {
    fn from(value: Vec<PathBuf>) -> Self {
        Self { inner: DataTransferDataInner::Paths(value) }
    }
}

impl From<String> for DataTransferData {
    fn from(value: String) -> Self {
        Self { inner: DataTransferDataInner::Plaintext(value) }
    }
}

impl DataTransferData {
    pub fn into_paths(self) -> io::Result<Vec<PathBuf>> {
        fn parse_paths(str: &str) -> io::Result<Vec<PathBuf>> {
            str.split(|c| c == '\n' || c == '\r')
                .map(|line| {
                    PathBuf::from_str(line)
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidFilename, err))
                })
                .collect()
        }

        match self.inner {
            DataTransferDataInner::Paths(paths) => Ok(paths),
            DataTransferDataInner::Plaintext(str) => parse_paths(&str),
            DataTransferDataInner::Bytes(mut buf_read) => {
                let mut string = String::new();
                buf_read.read_to_string(&mut string)?;
                parse_paths(&string)
            },
        }
    }

    pub fn into_string(self) -> io::Result<String> {
        match self.inner {
            DataTransferDataInner::Paths(_) => {
                // TODO: We could probably fudge this.
                Err(io::Error::new(io::ErrorKind::InvalidData, "Could not read as string"))
            },
            DataTransferDataInner::Plaintext(str) => Ok(str),
            DataTransferDataInner::Bytes(mut buf_read) => {
                let mut string = String::new();
                buf_read.read_to_string(&mut string)?;
                Ok(string)
            },
        }
    }

    pub fn into_reader(self) -> io::Result<impl BufRead> {
        match self.inner {
            DataTransferDataInner::Paths(_) => {
                // TODO: We could probably fudge this.
                Err(io::Error::new(io::ErrorKind::InvalidData, "Could not read"))
            },
            DataTransferDataInner::Plaintext(str) => {
                // TODO: We don't need to box here.
                Ok(Box::new(io::Cursor::new(str.into_bytes())) as Box<dyn BufRead>)
            },
            DataTransferDataInner::Bytes(buf_read) => Ok(buf_read),
        }
    }
}

#[derive(Clone)]
pub struct DataTransfer {
    id: DataTransferId,
    available_types: Arc<[String]>,
    fetch_data: Arc<dyn Fn(&str) -> io::Result<DataTransferData> + Send + Sync>,
}

impl PartialEq for DataTransfer {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.available_types == other.available_types
    }
}

impl Debug for DataTransfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataTransfer")
            .field("id", &self.id)
            .field("available_types", &self.available_types)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl DataTransfer {
    /// Testing function to create a [`DataTransfer`] from a set of paths.
    ///
    /// May eventually be exposed, but for now is kept private to reduce the API surface.
    pub(crate) fn from_paths(id: DataTransferId, paths: Vec<PathBuf>) -> Self {
        const URI_LIST_MIME_TYPE: &str = "text/uri-list";

        Self {
            id,
            available_types: vec![URI_LIST_MIME_TYPE].into(),
            fetch_data: Arc::new(|ty| match ty {
                URI_LIST_MIME_TYPE => Ok(paths.clone().into()),
                _ => Err(io::Error::new(io::ErrorKind::NotFound, "Invalid MIME type")),
            }),
        }
    }
}

impl DataTransfer {
    /// Create a new [`DataTransfer`] with a given ID and set of MIME types.
    pub fn new<I, F>(id: DataTransferId, available_types: I, fetch_data: F) -> Self
    where
        I: IntoIterator<Item = String>,
        F: Fn(&str) -> io::Result<DataTransferData> + Send + Sync + 'static,
    {
        fn normalize_mime_type(type_: String) -> String {
            let Ok(mime_type) = mime::Mime::from_str(&type_) else {
                return type_;
            };

            mime_type.essence_str().to_ascii_lowercase()
        }

        // Even though most platform implementations will be able to ensure
        // the required invariants before construction, we normalize within the
        // constructor to avoid exposing the precise inner types to the public
        // API.
        let available_types = available_types
            .into_iter()
            // First, normalize each MIME type to its canonical form.
            .map(normalize_mime_type)
            // Deduplicate and sort.
            .collect::<BTreeSet<_>>()
            .into_iter()
            // Finally, convert to an `Arc<[String]>`.
            .collect::<Vec<_>>()
            .into();

        Self { id, available_types, fetch_data: Arc::new(fetch_data) }
    }

    /// Display the list of all available MIME types.
    ///
    /// This is useful if more-complex MIME type matching is required, but for most cases
    /// [`has_type`](DataTransfer::has_type) should be used.
    pub fn available_types(&self) -> impl Iterator<Item = &str> {
        self.available_types.iter().map(AsRef::as_ref)
    }

    /// Fetch the data of the specified type.
    pub fn fetch(&self, mime_type: &str) -> io::Result<DataTransferData> {
        (self.fetch_data)(mime_type)
    }

    /// Check if the supplied MIME type is provided by this [`DataTransfer`].
    pub fn has_type(&self, type_: &str) -> bool {
        self.available_types.binary_search_by(|haystack| (&**haystack).cmp(type_)).is_err()
    }

    /// Get the ID of this [`DataTransfer`].
    pub const fn id(&self) -> DataTransferId {
        self.id
    }
}
