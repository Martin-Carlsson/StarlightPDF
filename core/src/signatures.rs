//! Digital signatures.
//!
//! # Certificates
//!
//! Certificates, certificate revocation lists and OCSP responses are DER encodings, concatenated.

use alloc::sync::Arc;

use crate::{Error, ErrorCode};

/// Options for [`Document::add_verification_data`](crate::document::Document::add_verification_data).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddVerificationDataOptions {
    /// [Certificates](crate::signatures#certificates).
    pub certificates: Vec<u8>,
    /// [Certificate revocation lists](crate::signatures#certificates).
    pub crls: Vec<u8>,
    /// [OCSP responses](crate::signatures#certificates).
    pub ocsp: Vec<u8>,
}

/// What changed after signing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChangeStatus {
    /// Only changes the signature permits.
    Allowed,
    /// Changes the signature forbids.
    Disallowed,
    /// Nothing.
    Unchanged,
}

/// A digest algorithm.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Digest {
    /// SHA-256.
    #[default]
    Sha256,
    /// SHA-384.
    Sha384,
    /// SHA-512.
    Sha512,
}

/// Options for [`PendingSignature::finalize`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FinalizeSignatureOptions {
    /// The signer's private key (PKCS #8, DER), used when `signature` is empty.
    pub private_key: Vec<u8>,
    /// The signer's signature over [`PendingSignature::to_sign`], or a timestamp token for `DocumentTimestamp`.
    pub signature: Vec<u8>,
    /// An RFC 3161 token over the signature; empty: none.
    pub timestamp: Vec<u8>,
}

/// A prepared signature waiting for the signer.
#[derive(Clone, Debug)]
pub struct PendingSignature {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl PendingSignature {
    /// Writes the signature into the prepared output.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::NotSeekable`].
    pub fn finalize(&self, options: &FinalizeSignatureOptions) -> Result<(), Error> {
        let _: &FinalizeSignatureOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "PendingSignature::finalize",
        ))
    }

    /// The bytes the signer signs (a smart card, HSM or remote service).
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::DocumentClosed`].
    pub fn to_sign(&self) -> Result<Vec<u8>, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "PendingSignature::to_sign",
        ))
    }
}

/// Options for [`Document::prepare_signature`](crate::document::Document::prepare_signature).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct PrepareSignatureOptions {
    /// The signer's [certificate](crate::signatures#certificates) chain, signer first.
    pub certificates: Vec<u8>,
    /// The signer's contact information.
    pub contact: String,
    /// The digest algorithm.
    pub digest: Digest,
    /// A signature field made with [`Page::add_field`](crate::page::Page::add_field) and
    /// [`FieldKind::Signature`](crate::forms::FieldKind::Signature); empty: a new invisible one.
    pub field: String,
    /// The kind.
    pub kind: SignatureKind,
    /// Where it was signed.
    pub location: String,
    /// Why it was signed.
    pub reason: String,
}

/// A certificate's revocation status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Revocation {
    /// Not revoked.
    Good,
    /// Revoked.
    Revoked,
    /// No revocation data.
    Unknown,
}

/// A signature in the document.
#[derive(Clone, Debug)]
pub struct Signature {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Signature {
    /// The signature's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<SignatureInfo, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Signature::info"))
    }

    /// Checks the signature, its chain and revocation.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn verify(&self, options: &VerifyOptions) -> Result<Verification, Error> {
        let _: &VerifyOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Signature::verify"))
    }
}

/// A signature's properties.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SignatureInfo {
    /// The signer's [certificate](crate::signatures#certificates) chain, signer first.
    pub certificates: Vec<u8>,
    /// The signer's contact information.
    pub contact: String,
    /// The digest algorithm.
    pub digest: Digest,
    /// The signature field.
    pub field: String,
    /// The kind.
    pub kind: SignatureKind,
    /// Where it was signed.
    pub location: String,
    /// Why it was signed.
    pub reason: String,
    /// The certificate subject.
    pub signer: String,
    /// When it was signed, a [date](crate#dates).
    pub signing_time: String,
    /// The timestamp token's [date](crate#dates); empty: none.
    pub timestamp_time: String,
}

/// The kind of a signature.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SignatureKind {
    /// An approval signature.
    #[default]
    Approval,
    /// A certification signature; no changes allowed.
    Certification,
    /// A certification signature; form filling and annotations allowed.
    CertificationAnnotations,
    /// A certification signature; form filling allowed.
    CertificationForms,
    /// A document timestamp (ISO 32000-2:2020, 12.8.5).
    DocumentTimestamp,
}

/// A document's signatures.
#[derive(Clone, Debug)]
pub struct Signatures {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Signatures {
    /// The next signature, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Signature>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Signatures::next"))
    }
}

/// A certificate, certificate revocation list or OCSP response stored for long-term validation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct VerificationItem {
    /// The DER encoding.
    pub data: Vec<u8>,
    /// What it is.
    pub kind: VerificationItemKind,
}

/// What a [`VerificationItem`] is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VerificationItemKind {
    /// A certificate.
    Certificate,
    /// A certificate revocation list.
    Crl,
    /// An OCSP response.
    Ocsp,
}

/// The items stored for long-term validation, one at a time.
#[derive(Clone, Debug)]
pub struct VerificationItems {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl VerificationItems {
    /// The next item, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<VerificationItem>, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "VerificationItems::next",
        ))
    }
}

/// A signature check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Verification {
    /// Changes after signing.
    pub changes: ChangeStatus,
    /// The signed bytes are unchanged.
    pub intact: bool,
    /// The signer certificate's revocation status.
    pub revocation: Revocation,
    /// The chain reaches a trusted root.
    pub trusted: bool,
}

/// Options for [`Signature::verify`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct VerifyOptions {
    /// [Certificate revocation lists](crate::signatures#certificates) beyond what the file holds.
    pub crls: Vec<u8>,
    /// [OCSP responses](crate::signatures#certificates) beyond what the file holds.
    pub ocsp: Vec<u8>,
    /// Trusted root [certificates](crate::signatures#certificates).
    pub roots: Vec<u8>,
    /// Validation [date](crate#dates); empty: now.
    pub time: String,
}
