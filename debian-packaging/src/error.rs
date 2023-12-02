// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/*! Error handling. */

use {simple_file_manifest::FileManifestError, thiserror::Error};

/// Primary crate error type.
#[derive(Debug, Error)]
pub enum DebianError {
    #[error("file manifest error: {0}")]
    FileManifestError(#[from] FileManifestError),

    #[error("URL error: {0:?}")]
    Url(#[from] url::ParseError),

    #[error("PGP error: {0:?}")]
    Pgp(#[from] pgp::errors::Error),

    #[error("date parsing error: {0:?}")]
    DateParse(#[from] mailparse::MailParseError),

    #[cfg(feature = "http")]
    #[error("HTTP error: {0:?}")]
    Reqwest(#[from] reqwest::Error),

    #[error("I/O error: {0:?}")]
    Io(#[from] std::io::Error),

    #[error("integer parsing error: {0:?}")]
    ParseInt(#[from] std::num::ParseIntError),

    #[error("invalid hex string (`{0}`) when parsing content digest: {0:?}")]
    ContentDigestBadHex(String, hex::FromHexError),

    #[error("control file parse error: {0}")]
    ControlParseError(String),

    #[error("Control file lacks a paragraph")]
    ControlFileNoParagraph,

    #[error("Control file not found")]
    ControlFileNotFound,

    #[error("cannot convert to simple field value since value contains line breaks")]
    ControlSimpleValueNoMultiline,

    #[error("required control paragraph field not found: {0}")]
    ControlRequiredFieldMissing(String),

    #[error("control field {0} can not be parsed as an integer: {0:?}")]
    ControlFieldIntParse(String, std::num::ParseIntError),

    #[error("failed to parse control field timestamp")]
    ControlFieldTimestampParse,

    #[error("missing field {0} in Package-List entry")]
    ControlPackageListMissingField(&'static str),

    #[error("expected 1 control paragraph in Debian source control file; got {0}")]
    DebianSourceControlFileParagraphMismatch(usize),

    #[error("unknown entry in binary package archive: {0}")]
    DebUnknownBinaryPackageEntry(String),

    #[error("unknown compression in deb archive file: {0}")]
    DebUnknownCompression(String),

    #[error("do not know how to construct repository reader from URL: {0}")]
    RepositoryReaderUnrecognizedUrl(String),

    #[error("do not know how to construct repository writer from URL: {0}")]
    RepositoryWriterUnrecognizedUrl(String),

    #[error("release file does not contain supported checksum flavor")]
    RepositoryReadReleaseNoKnownChecksum,

    #[error("could not find Contents indices entry in Release file")]
    RepositoryReadContentsIndicesEntryNotFound,

    #[error("could not find packages indices entry in Release file")]
    RepositoryReadPackagesIndicesEntryNotFound,

    #[error("could not find Sources indices entry in Release file")]
    RepositoryReadSourcesIndicesEntryNotFound,

    #[error("could not determine content digest of binary package")]
    RepositoryReadCouldNotDeterminePackageDigest,

    #[error("No packages indices for checksum {0}")]
    RepositoryNoPackagesIndices(&'static str),

    #[error("repository I/O error on path {0}: {1:?}")]
    RepositoryIoPath(String, std::io::Error),

    #[error("attempting to add package to undefined component: {0}")]
    RepositoryBuildUnknownComponent(String),

    #[error("attempting to add package to undefined architecture: {0}")]
    RepositoryBuildUnknownArchitecture(String),

    #[error("pool layout cannot be changed after content is indexed")]
    RepositoryBuildPoolLayoutImmutable,

    #[error(".deb not available: {0}")]
    RepositoryBuildDebNotAvailable(&'static str),

    #[error("expected 1 paragraph in control file; got {0}")]
    ReleaseControlParagraphMismatch(usize),

    #[error("digest missing from index entry")]
    ReleaseMissingDigest,

    #[error("size missing from index entry")]
    ReleaseMissingSize,

    #[error("path missing from index entry")]
    ReleaseMissingPath,

    #[error("index entry path unexpectedly has spaces: {0}")]
    ReleasePathWithSpaces(String),

    #[error("release indices file cannot be converted to the given type")]
    ReleaseIndicesEntryWrongType,

    #[error("No PGP signatures found")]
    ReleaseNoSignatures,

    #[error("No PGP signatures found from the specified key")]
    ReleaseNoSignaturesByKey,

    #[error("indices files not found in Release file")]
    ReleaseNoIndicesFiles,

    #[error("failed to parse dependency expression: {0}")]
    DependencyParse(String),

    #[error("unknown binary dependency field: {0}")]
    UnknownBinaryDependencyField(String),

    #[error("the epoch component has non-digit characters: {0}")]
    EpochNonNumeric(String),

    #[error("upstream_version component has illegal character: {0}")]
    UpstreamVersionIllegalChar(String),

    #[error("debian_revision component has illegal character: {0}")]
    DebianRevisionIllegalChar(String),

    #[error("unknown S3 region: {0}")]
    S3BadRegion(String),

    #[error("unknown verify behavior for null:// destination: {0}")]
    SinkWriterVerifyBehaviorUnknown(String),

    #[error("{0}")]
    Other(String),
}

/// Result wrapper for this crate.
pub type Result<T> = std::result::Result<T, DebianError>;
