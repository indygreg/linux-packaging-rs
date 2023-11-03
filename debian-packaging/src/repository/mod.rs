// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/*! Debian repositories.

A Debian repository is a collection of files holding packages and other
support primitives. See <https://wiki.debian.org/DebianRepository/Format>
for the canonical definition of a Debian repository.

Here's the concise version.

A Debian repository is a collection of files rooted at a given path or URL.
In Apt sources files, this is the value after lines beginning with `apt`. e.g.
`deb http://us.archive.ubuntu.com/ubuntu/ impish main` defines the repository
rooted at `http://us.archive.ubuntu.com/ubuntu/`.

Under the root are `dists/<distribution>/` directories. Each of these
directories (`<distribution>` can have `/` within them) has a `Release`
and/or `InRelease` file. These files serve as the main *index* for a given
*distribution*. (`InRelease` is the same as `Release` except it has a PGP
cleartext signature and may be encoded slightly differently as a result.)

Each `[In]Release` file defines metadata for a *distribution* as well as
provides a manifest of additional files further defining the content of the
*distribution*. These additional files define available binary packages (what
you typically install), sources packages, lists of which packages provide which
filenames, etc.

In addition to *distributions*, a repository also contains a *pool* where
non-distribution blob data is stored. This typically exists under the
`pool/` path relative to the repository root.

Since repositories are logically a virtual filesystem and can be backed
by any key-value blob store, we've abstracted repository I/O through a
series of traits.

[DataResolver] is a generic trait for providing path/key based I/O.

[RepositoryRootReader] describes an interface for reading from the root of
a repository. It is used to obtain and parse `[In]Release` files and to read
from the *pool*.

[ReleaseReader] describes an interface for reading from a *distribution*
and a parsed `[In]Release` file describing the distribution.

[RepositoryWriter] describes an interface for writing to a repository.

Concrete implementations of repositories exist in submodules. [http]
provides [http::HttpRepositoryClient], which implements [RepositoryRootReader]
and serves as the primary HTTP-based client. [filesystem] provides
[filesystem::FilesystemRepositoryReader] and [filesystem::FilesystemRepositoryWriter]
for reading and writing repositories using a local filesystem. [s3] provides
[s3::S3Writer].

A couple of special [RepositoryWriter] exist. [sink_writer::SinkWriter] provides a writer
that will send its content to a black hole. It can be used for testing writing without
actually performing writes. [proxy_writer::ProxyWriter] proxies an inner writer and
can override behavior on certain I/O operations.

Modules like [contents] and [release] define primitives encountered in
repositories, such as `[In]Release` files.

The [builder] module contains functionality for creating/publishing
repositories.
*/

use std::fmt::Formatter;
use {
    crate::{
        binary_package_control::BinaryPackageControlFile,
        binary_package_list::BinaryPackageList,
        control::ControlParagraphAsyncReader,
        deb::reader::BinaryPackageReader,
        debian_source_control::{DebianSourceControlFile, DebianSourceControlFileFetch},
        debian_source_package_list::DebianSourcePackageList,
        error::{DebianError, Result},
        io::{drain_reader, Compression, ContentDigest, DataResolver},
        repository::{
            contents::{ContentsFile, ContentsFileAsyncReader},
            release::{
                ChecksumType, ClassifiedReleaseFileEntry, ContentsFileEntry, PackagesFileEntry,
                ReleaseFile, SourcesFileEntry,
            },
        },
    },
    async_trait::async_trait,
    futures::{AsyncRead, AsyncReadExt, StreamExt, TryStreamExt},
    std::{borrow::Cow, collections::HashMap, ops::Deref, pin::Pin, str::FromStr},
};

pub mod builder;
pub mod contents;
pub mod copier;
pub mod filesystem;
#[cfg(feature = "http")]
pub mod http;
pub mod proxy_writer;
pub mod release;
#[cfg(feature = "s3")]
pub mod s3;
pub mod sink_writer;

/// Describes how to fetch a binary package from a repository.
#[derive(Clone, Debug)]
pub struct BinaryPackageFetch<'a> {
    /// The binary package control paragraph from which this entry came.
    pub control_file: BinaryPackageControlFile<'a>,
    /// The relative path of this binary package.
    ///
    /// Corresponds to the `Filename` field.
    pub path: String,
    /// The expected size of the retrieved file.
    pub size: u64,
    /// The expected content digest of the retrieved file.
    pub digest: ContentDigest,
}

/// Describes how to fetch a source package from a repository.
pub struct SourcePackageFetch<'a> {
    /// The control file from which this these fetches were derived.
    pub control_file: DebianSourceControlFile<'a>,
    /// Fetch instruction for a file in this package.
    fetch: DebianSourceControlFileFetch,
}

impl<'a> Deref for SourcePackageFetch<'a> {
    type Target = DebianSourceControlFileFetch;

    fn deref(&self) -> &Self::Target {
        &self.fetch
    }
}

/// Debian repository reader bound to the root of the repository.
///
/// This trait facilitates access to *pool* as well as to multiple
/// *releases* within the repository.
#[async_trait]
pub trait RepositoryRootReader: DataResolver + Sync {
    /// Obtain the URL to which this reader is bound.  
    fn url(&self) -> Result<url::Url>;

    /// Obtain a [ReleaseReader] for a given distribution.
    ///
    /// This assumes either an `InRelease` or `Release` file is located in `dists/{distribution}/`.
    /// This is the case for most repositories.
    async fn release_reader(&self, distribution: &str) -> Result<Box<dyn ReleaseReader>> {
        self.release_reader_with_distribution_path(&format!(
            "dists/{}",
            distribution.trim_matches('/')
        ))
        .await
    }

    /// Obtain a [ReleaseReader] given a distribution path.
    ///
    /// Typically distributions exist at `dists/<distribution>/`. However, this may not
    /// always be the case. This method allows explicitly passing in the relative path
    /// holding the `InRelease` file.
    async fn release_reader_with_distribution_path(
        &self,
        path: &str,
    ) -> Result<Box<dyn ReleaseReader>>;

    /// Fetch and parse an `InRelease` file at the relative path specified.
    ///
    /// `path` is typically a value like `dists/<distribution>/InRelease`. e.g.
    /// `dists/bullseye/InRelease`.
    ///
    /// The default implementation of this trait should be sufficient for most types.
    async fn fetch_inrelease(&self, path: &str) -> Result<ReleaseFile<'static>> {
        let mut reader = self.get_path(path).await?;

        let mut data = vec![];
        reader.read_to_end(&mut data).await?;

        Ok(ReleaseFile::from_armored_reader(std::io::Cursor::new(
            data,
        ))?)
    }

    /// Fetch and parse an `Release` file at the relative path specified.
    ///
    /// `path` is typically a value like `dists/<distribution>/Release`. e.g.
    /// `dists/bullseye/Release`.
    ///
    /// The default implementation of this trait should be sufficient for most types.
    async fn fetch_release(&self, path: &str) -> Result<ReleaseFile<'static>> {
        let mut reader = self.get_path(path).await?;

        let mut data = vec![];
        reader.read_to_end(&mut data).await?;

        Ok(ReleaseFile::from_reader(std::io::Cursor::new(data))?)
    }
    /// Fetch and parse either an `InRelease` or `Release` file at the relative path specified.
    ///
    /// First attempt to use the more modern `InRelease` file, fall back to `Release`
    ///
    /// The default implementation of this trait should be sufficient for most types.
    async fn fetch_inrelease_or_release(
        &self,
        inrelease_path: &str,
        release_path: &str,
    ) -> Result<ReleaseFile<'static>> {
        match self.fetch_inrelease(inrelease_path).await {
            Ok(release) => Ok(release),
            Err(DebianError::RepositoryIoPath(_, e))
                if e.kind() == std::io::ErrorKind::NotFound =>
            {
                self.fetch_release(release_path).await
            }
            Err(e) => Err(e),
        }
    }

    /// Fetch a binary package given a [BinaryPackageFetch] instruction.
    ///
    /// Returns a generic [AsyncRead] to obtain the raw file content.
    async fn fetch_binary_package_generic<'fetch>(
        &self,
        fetch: BinaryPackageFetch<'fetch>,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
        self.get_path_with_digest_verification(&fetch.path, fetch.size, fetch.digest)
            .await
    }

    /// Fetch a binary package given a [BinaryPackageFetch] instruction.
    ///
    /// Returns a [BinaryPackageReader] capable of parsing the package.
    ///
    /// Due to limitations in [BinaryPackageReader], the entire package content is buffered
    /// in memory and isn't read lazily.
    async fn fetch_binary_package_deb_reader<'fetch>(
        &self,
        fetch: BinaryPackageFetch<'fetch>,
    ) -> Result<BinaryPackageReader<std::io::Cursor<Vec<u8>>>> {
        let mut reader = self.fetch_binary_package_generic(fetch).await?;
        // TODO implement an async reader.
        let mut buf = vec![];
        reader.read_to_end(&mut buf).await?;

        Ok(BinaryPackageReader::new(std::io::Cursor::new(buf))?)
    }

    /// Fetch a source package file given a [SourcePackageFetch] instruction.
    ///
    /// Returns a generic [AsyncRead] to obtain the raw file content.
    async fn fetch_source_package_generic<'fetch>(
        &self,
        fetch: SourcePackageFetch<'fetch>,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
        self.get_path_with_digest_verification(&fetch.path, fetch.size, fetch.digest.clone())
            .await
    }
}

/// Provides a transport-agnostic mechanism for reading from a parsed `[In]Release` file.
#[async_trait]
pub trait ReleaseReader: DataResolver + Sync {
    /// Obtain the base URL to which this instance is bound.
    fn url(&self) -> Result<url::Url>;

    /// Obtain the path relative to the repository root this instance is bound to.
    ///
    /// e.g. `dists/bullseye`.
    ///
    /// Implementations must not return a string with a leading or trailing `/`.
    fn root_relative_path(&self) -> &str;

    /// Obtain the parsed `[In]Release` file from which this reader is derived.
    fn release_file(&self) -> &ReleaseFile<'_>;

    /// Obtain the checksum flavor of content to retrieve.
    ///
    /// By default, this will prefer the strongest known checksum advertised in the
    /// release file.
    fn retrieve_checksum(&self) -> Result<ChecksumType> {
        let release = self.release_file();

        let checksum = &[ChecksumType::Sha256, ChecksumType::Sha1, ChecksumType::Md5]
            .iter()
            .find(|variant| release.field(variant.field_name()).is_some())
            .ok_or(DebianError::RepositoryReadReleaseNoKnownChecksum)?;

        Ok(**checksum)
    }

    /// Obtain the preferred compression format to retrieve index files in.
    fn preferred_compression(&self) -> Compression;

    /// Set the preferred compression format for retrieved index files.
    ///
    /// Index files are often published in multiple compression formats, including no
    /// compression. This function can be used to instruct the reader which compression
    /// format to prefer.
    fn set_preferred_compression(&mut self, compression: Compression);

    /// Obtain [ClassifiedReleaseFileEntry] within the parsed `Release` file.
    fn classified_indices_entries(&self) -> Result<Vec<ClassifiedReleaseFileEntry<'_>>> {
        self.release_file()
            .iter_classified_index_files(self.retrieve_checksum()?)
            .ok_or(DebianError::ReleaseNoIndicesFiles)?
            .collect::<Result<Vec<_>>>()
    }

    /// Obtain parsed `Packages` file entries within this Release file.
    ///
    /// Only entries for the checksum as defined by [Self::retrieve_checksum()] are returned.
    ///
    /// There may be multiple entries for a given logical `Packages` file corresponding
    /// to different compression formats. Use [Self::packages_entry()] to resolve the entry
    /// for the `Packages` file for the preferred configuration.
    fn packages_indices_entries(&self) -> Result<Vec<PackagesFileEntry<'_>>> {
        Ok(
            if let Some(entries) = self
                .release_file()
                .iter_packages_indices(self.retrieve_checksum()?)
            {
                entries.collect::<Result<Vec<_>>>()?
            } else {
                vec![]
            },
        )
    }

    /// Like [Self::packages_indices_entries()] except it deduplicates entries.
    ///
    /// If there are multiple entries for a `Packages` file with varying compression, the most
    /// preferred compression format is returned.
    fn packages_indices_entries_preferred_compression(&self) -> Result<Vec<PackagesFileEntry<'_>>> {
        let mut entries = HashMap::new();

        for entry in self.packages_indices_entries()? {
            entries
                .entry((
                    entry.component.clone(),
                    entry.architecture.clone(),
                    entry.is_installer,
                ))
                .or_insert_with(Vec::new)
                .push(entry);
        }

        entries
            .into_values()
            .map(|candidates| {
                if let Some(entry) = candidates
                    .iter()
                    .find(|entry| entry.compression == self.preferred_compression())
                {
                    Ok(entry.clone())
                } else {
                    for compression in Compression::default_preferred_order() {
                        if let Some(entry) = candidates
                            .iter()
                            .find(|entry| entry.compression == compression)
                        {
                            return Ok(entry.clone());
                        }
                    }

                    Err(DebianError::RepositoryReadPackagesIndicesEntryNotFound)
                }
            })
            .collect::<Result<Vec<_>>>()
    }

    /// Resolve indices for `Contents` files.
    ///
    /// Only entries for the checksum as defined by [Self::retrieve_checksum()] are returned.
    ///
    /// Multiple entries for the same logical file with varying compression formats may be
    /// returned.
    fn contents_indices_entries(&self) -> Result<Vec<ContentsFileEntry<'_>>> {
        Ok(
            if let Some(entries) = self
                .release_file()
                .iter_contents_indices(self.retrieve_checksum()?)
            {
                entries.collect::<Result<Vec<_>>>()?
            } else {
                vec![]
            },
        )
    }

    /// Resolve indices for `Sources` file.
    ///
    /// Only entries for the checksum as defined by [Self::retrieve_checksum()] are returned.
    ///
    /// Multiple entries for the same logical file with varying compression formats may be
    /// returned.
    fn sources_indices_entries(&self) -> Result<Vec<SourcesFileEntry<'_>>> {
        Ok(
            if let Some(entries) = self
                .release_file()
                .iter_sources_indices(self.retrieve_checksum()?)
            {
                entries.collect::<Result<Vec<_>>>()?
            } else {
                vec![]
            },
        )
    }

    /// Like [Self::sources_indices_entries] except it deduplicates entries.
    ///
    /// If there are multiple entries for a `Sources` file with varying compression, the most
    /// preferred compression format is returned.
    fn sources_indices_entries_preferred_compression(&self) -> Result<Vec<SourcesFileEntry<'_>>> {
        let mut entries = HashMap::new();

        for entry in self.sources_indices_entries()? {
            entries
                .entry(entry.component.clone())
                .or_insert_with(Vec::new)
                .push(entry);
        }

        entries
            .into_values()
            .map(|candidates| {
                if let Some(entry) = candidates
                    .iter()
                    .find(|entry| entry.compression == self.preferred_compression())
                {
                    Ok(entry.clone())
                } else {
                    for compression in Compression::default_preferred_order() {
                        if let Some(entry) = candidates
                            .iter()
                            .find(|entry| entry.compression == compression)
                        {
                            return Ok(entry.clone());
                        }
                    }

                    Err(DebianError::RepositoryReadPackagesIndicesEntryNotFound)
                }
            })
            .collect::<Result<Vec<_>>>()
    }

    /// Resolve a reference to a `Packages` file to fetch given search criteria.
    ///
    /// This will find all entries defining the desired `Packages` file. It will filter
    /// through the [ChecksumType] as defined by [Self::retrieve_checksum()] and will prioritize
    /// the compression format according to [Self::preferred_compression()].
    fn packages_entry(
        &self,
        component: &str,
        architecture: &str,
        is_installer: bool,
    ) -> Result<PackagesFileEntry<'_>> {
        self.packages_indices_entries_preferred_compression()?
            .into_iter()
            .find(|entry| {
                entry.component == component
                    && entry.architecture == architecture
                    && entry.is_installer == is_installer
            })
            .ok_or(DebianError::RepositoryReadPackagesIndicesEntryNotFound)
    }

    /// Fetch and parse a `Packages` file described by a [PackagesFileEntry].
    async fn resolve_packages_from_entry<'entry, 'slf: 'entry>(
        &'slf self,
        entry: &'entry PackagesFileEntry<'slf>,
    ) -> Result<BinaryPackageList<'static>> {
        let release = self.release_file();

        let path = if release.acquire_by_hash().unwrap_or_default() {
            entry.by_hash_path()
        } else {
            entry.path.to_string()
        };

        let mut reader = ControlParagraphAsyncReader::new(futures::io::BufReader::new(
            self.get_path_decoded_with_digest_verification(
                &path,
                entry.compression,
                entry.size,
                entry.digest.clone(),
            )
            .await?,
        ));

        let mut res = BinaryPackageList::default();

        while let Some(paragraph) = reader.read_paragraph().await? {
            res.push(BinaryPackageControlFile::from(paragraph));
        }

        Ok(res)
    }

    /// Resolve packages given parameters to resolve a `Packages` file.
    async fn resolve_packages(
        &self,
        component: &str,
        arch: &str,
        is_installer: bool,
    ) -> Result<BinaryPackageList<'static>> {
        let entry = self.packages_entry(component, arch, is_installer)?;

        self.resolve_packages_from_entry(&entry).await
    }

    /// Retrieve fetch instructions for binary packages.
    ///
    /// The caller can specify a filter function to choose which packages to retrieve.
    /// Filtering works in 2 stages.
    ///
    /// First, `packages_file_filter` is called with each [PackagesFileEntry] defining
    /// a `Packages*` file. If the filter returns true, this list of packages will be
    /// retrieved and expanded.
    ///
    /// Second, `binary_package_filter` is called for each binary package entry seen
    /// in parsed `Packages*` files. If the function returns true, this binary package
    /// will be retrieved.
    ///
    /// The emitted values can be fed into [RepositoryRootReader::fetch_binary_package_generic()]
    /// and [RepositoryRootReader::fetch_binary_package_deb_reader()] to fetch the binary package
    /// content.
    async fn resolve_package_fetches(
        &self,
        packages_file_filter: Box<dyn (Fn(PackagesFileEntry) -> bool) + Send>,
        binary_package_filter: Box<dyn (Fn(BinaryPackageControlFile) -> bool) + Send>,
        threads: usize,
    ) -> Result<Vec<BinaryPackageFetch<'_>>> {
        let packages_entries = self.packages_indices_entries_preferred_compression()?;

        let fs = packages_entries
            .iter()
            .filter(|entry| packages_file_filter((*entry).clone()))
            .map(|entry| self.resolve_packages_from_entry(entry))
            .collect::<Vec<_>>();

        let mut packages_fs = futures::stream::iter(fs).buffer_unordered(threads);

        let mut fetches = vec![];

        while let Some(pl) = packages_fs.try_next().await? {
            for cf in pl.into_iter() {
                // Needed by IDE for type hinting for some reason.
                let cf: BinaryPackageControlFile = cf;

                if binary_package_filter(cf.clone()) {
                    let path = cf.required_field_str("Filename")?.to_string();

                    let size = cf.field_u64("Size").ok_or_else(|| {
                        DebianError::ControlRequiredFieldMissing("Size".to_string())
                    })??;

                    let digest = ChecksumType::preferred_order()
                        .find_map(|checksum| {
                            cf.field_str(checksum.field_name()).map(|hex_digest| {
                                ContentDigest::from_hex_digest(checksum, hex_digest)
                            })
                        })
                        .ok_or(DebianError::RepositoryReadCouldNotDeterminePackageDigest)??;

                    fetches.push(BinaryPackageFetch {
                        control_file: cf,
                        path,
                        size,
                        digest,
                    });
                }
            }
        }

        Ok(fetches)
    }

    /// Resolve the [SourcesFileEntry] for a given component.
    ///
    /// This returns the entry variant that is preferred given digest and compression
    /// settings. If no entry is found, [DebianError::RepositoryReadSourcesIndicesEntryNotFound]
    /// is returned.
    fn sources_entry(&self, component: &str) -> Result<SourcesFileEntry<'_>> {
        self.sources_indices_entries_preferred_compression()?
            .into_iter()
            .find(|entry| entry.component == component)
            .ok_or(DebianError::RepositoryReadSourcesIndicesEntryNotFound)
    }

    /// Fetch a `Sources` file and parse source package entries inside.
    ///
    /// The file to fetch is specified from a [SourcesFileEntry] describing it.
    async fn resolve_sources_from_entry<'entry, 'slf: 'entry>(
        &'slf self,
        entry: &'entry SourcesFileEntry<'slf>,
    ) -> Result<DebianSourcePackageList<'static>> {
        let release = self.release_file();

        let path = if release.acquire_by_hash().unwrap_or_default() {
            entry.by_hash_path()
        } else {
            entry.path.to_string()
        };

        let mut reader = ControlParagraphAsyncReader::new(futures::io::BufReader::new(
            self.get_path_decoded_with_digest_verification(
                &path,
                entry.compression,
                entry.size,
                entry.digest.clone(),
            )
            .await?,
        ));

        let mut res = DebianSourcePackageList::default();

        while let Some(paragraph) = reader.read_paragraph().await? {
            res.push(paragraph.into());
        }

        Ok(res)
    }

    /// Fetch a `Sources` file for the given component and parse source package entries inside.
    ///
    /// This will call [Self::sources_entry] to resolve the [SourcesFileEntry] for the given
    /// `component` then will call [Self::resolve_sources_from_entry] to fetch and parse it.
    async fn resolve_sources(&self, component: &str) -> Result<DebianSourcePackageList<'static>> {
        let entry = self.sources_entry(component)?;

        self.resolve_sources_from_entry(&entry).await
    }

    /// Resolves [SourcePackageFetch] for describing files to fetch for source packages.
    ///
    /// The caller specifies filter functions to choose which source packages' files to
    /// retrieve. Filtering works in 2 stages.
    ///
    /// First, `sources_filter_filter` is called with each [SourcesFileEntry] defining a
    /// `Sources` file. If the filter returns true, this list of packages will be retrieved
    /// and expanded.
    ///
    /// Second, `source_package_filter` is called for each source package entry seen in
    /// parsed `Sources` files. If the function returns true, the instructions for fetching
    /// the files comprising this source package will returned.
    ///
    /// The returned [SourcePackageFetch] can be fed into
    /// [RepositoryRootReader::fetch_source_package_generic()] to retrieve the file content.
    async fn resolve_source_fetches(
        &self,
        sources_file_filter: Box<dyn (Fn(SourcesFileEntry) -> bool) + Send>,
        source_package_filter: Box<dyn (Fn(DebianSourceControlFile) -> bool) + Send>,
        threads: usize,
    ) -> Result<Vec<SourcePackageFetch<'_>>> {
        let sources_entries = self.sources_indices_entries_preferred_compression()?;

        let fs = sources_entries
            .iter()
            .filter(|entry| sources_file_filter((*entry).clone()))
            .map(|entry| self.resolve_sources_from_entry(entry))
            .collect::<Vec<_>>();

        let mut sources_fs = futures::stream::iter(fs).buffer_unordered(threads);

        let mut fetches = vec![];

        while let Some(pl) = sources_fs.try_next().await? {
            for cf in pl.into_iter() {
                if source_package_filter(cf.clone_no_signatures()) {
                    for fetch in cf.file_fetches(self.retrieve_checksum()?)? {
                        let fetch = fetch?;

                        fetches.push(SourcePackageFetch {
                            control_file: cf.clone_no_signatures(),
                            fetch,
                        });
                    }
                }
            }
        }

        Ok(fetches)
    }

    /// Resolve a reference to a `Contents` file to fetch given search criteria.
    ///
    /// This will attempt to find the entry for a `Contents` file given search criteria.
    fn contents_entry(
        &self,
        component: Option<&str>,
        architecture: &str,
        is_installer: bool,
    ) -> Result<ContentsFileEntry> {
        let component = component.map(Cow::from);

        let entries = self
            .contents_indices_entries()?
            .into_iter()
            .filter(|entry| {
                entry.component == component
                    && entry.architecture == architecture
                    && entry.is_installer == is_installer
            })
            .collect::<Vec<_>>();

        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.compression == self.preferred_compression())
        {
            Ok(entry.clone())
        } else {
            for compression in Compression::default_preferred_order() {
                if let Some(entry) = entries
                    .iter()
                    .find(|entry| entry.compression == compression)
                {
                    return Ok(entry.clone());
                }
            }

            Err(DebianError::RepositoryReadContentsIndicesEntryNotFound)
        }
    }

    async fn resolve_contents(
        &self,
        component: Option<&str>,
        architecture: &str,
        is_installer: bool,
    ) -> Result<ContentsFile> {
        let release = self.release_file();
        let entry = self.contents_entry(component, architecture, is_installer)?;

        let path = if release.acquire_by_hash().unwrap_or_default() {
            entry.by_hash_path()
        } else {
            entry.path.to_string()
        };

        let reader = self
            .get_path_decoded_with_digest_verification(
                &path,
                entry.compression,
                entry.size,
                entry.digest.clone(),
            )
            .await?;

        let mut reader = ContentsFileAsyncReader::new(futures::io::BufReader::new(reader));
        reader.read_all().await?;

        let (contents, reader) = reader.consume();

        drain_reader(reader)
            .await
            .map_err(|e| DebianError::RepositoryIoPath(path, e))?;

        Ok(contents)
    }
}

/// Describes a repository path verification state.
#[derive(Clone, Copy, Debug)]
pub enum RepositoryPathVerificationState {
    /// The path exists but its integrity was not verified.
    ExistsNoIntegrityCheck,
    /// The path exists and its integrity was verified.
    ExistsIntegrityVerified,
    /// The path exists and its integrity didn't match expectations.
    ExistsIntegrityMismatch,
    /// The path is missing.
    Missing,
}

/// Represents the result of a repository path verification check.
#[derive(Clone, Debug)]
pub struct RepositoryPathVerification<'a> {
    /// The path that was tested.
    pub path: &'a str,
    /// The state of the path.
    pub state: RepositoryPathVerificationState,
}

impl<'a> std::fmt::Display for RepositoryPathVerification<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.state {
            RepositoryPathVerificationState::ExistsNoIntegrityCheck => {
                write!(f, "{} exists (no integrity check performed)", self.path)
            }
            RepositoryPathVerificationState::ExistsIntegrityVerified => {
                write!(f, "{} exists (integrity verified)", self.path)
            }
            RepositoryPathVerificationState::ExistsIntegrityMismatch => {
                write!(f, "{} exists (integrity mismatch!)", self.path)
            }
            RepositoryPathVerificationState::Missing => {
                write!(f, "{} missing", self.path)
            }
        }
    }
}

/// A phase during a repository copy operation.
#[derive(Clone, Copy, Debug)]
pub enum CopyPhase {
    BinaryPackages,
    InstallerBinaryPackages,
    Sources,
    Installers,
    ReleaseIndices,
    ReleaseFiles,
}

impl std::fmt::Display for CopyPhase {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::BinaryPackages => "binary packages",
                Self::InstallerBinaryPackages => "installer binary packages",
                Self::Sources => "sources",
                Self::Installers => "installers",
                Self::ReleaseIndices => "release indices",
                Self::ReleaseFiles => "release files",
            }
        )
    }
}

/// Represents a repository publishing event.
///
/// Instances are sent to callbacks during repository writing to inform of activity.
pub enum PublishEvent {
    ResolvedPoolArtifacts(usize),

    /// A pool artifact with the given path is current and was not updated.
    PoolArtifactCurrent(String),

    /// A pool artifact with the given path is missing and will be created.
    PoolArtifactMissing(String),

    /// Total number of pool artifacts to publish.
    PoolArtifactsToPublish(usize),

    /// A pool artifact with the given path and size was created.
    PoolArtifactCreated(String, u64),

    /// The path to an index file to write.
    IndexFileToWrite(String),

    /// An index file that was written.
    IndexFileWritten(String, u64),

    /// A path is being verified.
    VerifyingDestinationPath(String),

    /// A phase in a copy operation has begin.
    CopyPhaseBegin(CopyPhase),

    /// A phase in a copy operation has finished.
    CopyPhaseEnd(CopyPhase),

    /// Copying a path from a source to a destination.
    CopyingPath(String, String),

    /// Copying an indices file but the source wasn't found.
    CopyIndicesPathNotFound(String),

    /// A path was copied.
    PathCopied(String, u64),

    /// A path copy was a no-op.
    PathCopyNoop(String),

    /// Begin a write sequence where we will write N total bytes.
    WriteSequenceBeginWithTotalBytes(u64),

    /// Report that N bytes have been written as part of a write operation.
    WriteSequenceProgressBytes(u64),

    /// Report the conclusion of a logical write sequence.
    WriteSequenceFinished,
}

impl std::fmt::Display for PublishEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResolvedPoolArtifacts(count) => {
                write!(f, "resolved {} needed pool artifacts", count)
            }
            Self::PoolArtifactCurrent(path) => {
                write!(f, "pool path {} is present", path)
            }
            Self::PoolArtifactMissing(path) => {
                write!(f, "pool path {} will be written", path)
            }
            Self::PoolArtifactsToPublish(count) => {
                write!(f, "{} pool artifacts will be written", count)
            }
            Self::PoolArtifactCreated(path, size) => {
                write!(f, "wrote {} bytes to {}", size, path)
            }
            Self::IndexFileToWrite(path) => {
                write!(f, "index file {} will be written", path)
            }
            Self::IndexFileWritten(path, size) => {
                write!(f, "wrote {} bytes to {}", size, path)
            }
            Self::VerifyingDestinationPath(path) => {
                write!(f, "verifying destination path {}", path)
            }
            Self::CopyPhaseBegin(phase) => {
                write!(f, "beginning copying of {}", phase)
            }
            Self::CopyPhaseEnd(phase) => {
                write!(f, "finished copying of {}", phase)
            }
            Self::CopyingPath(source, dest) => {
                write!(f, "copying {} to {}", source, dest)
            }
            Self::CopyIndicesPathNotFound(path) => {
                write!(
                    f,
                    "copying indices file {} failed because it wasn't found",
                    path
                )
            }
            Self::PathCopied(path, size) => {
                write!(f, "copied {} bytes to {}", size, path)
            }
            Self::PathCopyNoop(path) => {
                write!(f, "copy of {} was a no-op", path)
            }
            Self::WriteSequenceBeginWithTotalBytes(_)
            | Self::WriteSequenceProgressBytes(_)
            | Self::WriteSequenceFinished => Ok(()),
        }
    }
}

impl PublishEvent {
    /// Whether this even contains a meaningful log message.
    pub fn is_loggable(&self) -> bool {
        !self.is_progress()
    }

    /// Whether this is a progress update.
    pub fn is_progress(&self) -> bool {
        matches!(
            self,
            Self::WriteSequenceBeginWithTotalBytes(_)
                | Self::WriteSequenceProgressBytes(_)
                | Self::WriteSequenceFinished
        )
    }
}

#[derive(Clone, Debug)]
pub struct RepositoryWrite<'a> {
    /// The path that was written.
    pub path: Cow<'a, str>,
    /// The number of bytes written.
    pub bytes_written: u64,
}

/// Describes the result of a repository write operation.
pub enum RepositoryWriteOperation<'a> {
    /// A path was written.
    PathWritten(RepositoryWrite<'a>),
    /// The operation didn't do anything meaningful.
    Noop(Cow<'a, str>, u64),
}

impl<'a> RepositoryWriteOperation<'a> {
    pub fn bytes_written(&self) -> u64 {
        match self {
            Self::PathWritten(write) => write.bytes_written,
            Self::Noop(_, size) => *size,
        }
    }
}

/// An interface for writing to a repository.
///
/// From the perspective of this trait, writing to a repository is a matter of
/// providing I/O for testing for path/key existence/integrity and storing new
/// data under a path/key. Additional logic about what to write where is
/// implemented elsewhere.
#[async_trait]
pub trait RepositoryWriter: Sync {
    /// Verify the existence of a path with optional content integrity checking.
    ///
    /// If the size and digest are [Some] implementations *may* perform additional
    /// content integrity verification. Or they may not. They should not lie about
    /// whether integrity verification was performed in the returned value, however.
    async fn verify_path<'path>(
        &self,
        path: &'path str,
        expected_content: Option<(u64, ContentDigest)>,
    ) -> Result<RepositoryPathVerification<'path>>;

    /// Write data to a given path.
    ///
    /// The data to write is provided by an [AsyncRead] reader.
    async fn write_path<'path, 'reader>(
        &self,
        path: Cow<'path, str>,
        reader: Pin<Box<dyn AsyncRead + Send + 'reader>>,
    ) -> Result<RepositoryWrite<'path>>;

    /// Copy a path from a reader to this writer.
    ///
    /// The source reader is a [RepositoryRootReader] and the path is relative to the repository
    /// root.
    ///
    /// The default implementation verifies the integrity of the destination and will no-op if
    /// the desired content is already present.
    ///
    /// Implementations of this trait may have a custom implementation that changes semantics.
    /// For example, a writer could operate in a dry-run mode where it doesn't actually attempt
    /// any I/O. Custom implementations should call `progress_cb` with events, as appropriate.
    async fn copy_from<'path>(
        &self,
        reader: &dyn RepositoryRootReader,
        source_path: Cow<'path, str>,
        expected_content: Option<(u64, ContentDigest)>,
        dest_path: Cow<'path, str>,
        progress_cb: &Option<Box<dyn Fn(PublishEvent) + Sync>>,
    ) -> Result<RepositoryWriteOperation<'path>> {
        if let Some(cb) = progress_cb {
            cb(PublishEvent::VerifyingDestinationPath(
                dest_path.to_string(),
            ));
        }

        let verification = self
            .verify_path(dest_path.as_ref(), expected_content.clone())
            .await?;

        if matches!(
            verification.state,
            RepositoryPathVerificationState::ExistsIntegrityVerified
        ) {
            return Ok(RepositoryWriteOperation::Noop(
                dest_path,
                if let Some((size, _)) = expected_content {
                    size
                } else {
                    0
                },
            ));
        }

        if let Some(cb) = progress_cb {
            cb(PublishEvent::CopyingPath(
                source_path.to_string(),
                dest_path.to_string(),
            ));
        }

        let reader = if let Some((size, digest)) = expected_content {
            reader
                .get_path_with_digest_verification(source_path.as_ref(), size, digest)
                .await?
        } else {
            reader.get_path(source_path.as_ref()).await?
        };

        let write = self.write_path(dest_path, reader).await?;

        Ok(RepositoryWriteOperation::PathWritten(write))
    }
}

/// Construct a [RepositoryRootReader] from a string/URL.
///
/// If the string contains `://` it will be parsed as a URL. `file://`, `http://`,
/// and `https://` are recognized.
///
/// Otherwise the string will be interpreted as a filesystem path. No test for whether
/// the repository exists is performed.
pub fn reader_from_str(s: impl ToString) -> Result<Box<dyn RepositoryRootReader>> {
    let s = s.to_string();

    if s.contains("://") {
        let url = url::Url::parse(&s)?;

        match url.scheme() {
            "file" => Ok(Box::new(filesystem::FilesystemRepositoryReader::new(
                url.to_file_path()
                    .expect("path conversion should always work for file://"),
            ))),
            #[cfg(feature = "http")]
            "http" | "https" => Ok(Box::new(http::HttpRepositoryClient::new(url)?)),
            _ => Err(DebianError::RepositoryReaderUnrecognizedUrl(s)),
        }
    } else {
        // Assume a filesystem path.
        Ok(Box::new(filesystem::FilesystemRepositoryReader::new(s)))
    }
}

/// Construct a [RepositoryWriter] from a string/URL.
///
/// If the string contains `://` it will be parsed as a URL. `file://`, `null://`, and `s3://` are
/// recognized.
///
/// Otherwise the string will be interpreted as a filesystem path. No test for
/// whether the repository exists is performed.
pub async fn writer_from_str(s: impl ToString) -> Result<Box<dyn RepositoryWriter>> {
    let s = s.to_string();

    if s.contains("://") {
        let url = url::Url::parse(&s)?;

        match url.scheme() {
            "file" => Ok(Box::new(filesystem::FilesystemRepositoryWriter::new(
                url.to_file_path()
                    .expect("path conversion should always work for file://"),
            ))),
            "null" => {
                let mut writer = sink_writer::SinkWriter::default();

                let behavior = match url.host_str() {
                    Some(s) => sink_writer::SinkWriterVerifyBehavior::from_str(s)?,
                    None => sink_writer::SinkWriterVerifyBehavior::Missing,
                };

                writer.set_verify_behavior(behavior);

                Ok(Box::new(writer))
            }
            #[cfg(feature = "s3")]
            "s3" => {
                let path = url.path();

                if let Some((bucket, prefix)) = path.trim_matches('/').split_once('/') {
                    let region = s3::get_bucket_region(bucket).await?;

                    Ok(Box::new(s3::S3Writer::new(region, bucket, Some(prefix))))
                } else {
                    let region = s3::get_bucket_region(path).await?;

                    Ok(Box::new(s3::S3Writer::new(region, path, None)))
                }
            }
            _ => Err(DebianError::RepositoryWriterUnrecognizedUrl(s)),
        }
    } else {
        Ok(Box::new(filesystem::FilesystemRepositoryWriter::new(s)))
    }
}
