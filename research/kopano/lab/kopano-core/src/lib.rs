//! Kopano prototype core (research lab, not product code).
//!
//! Application-neutral: it knows objects, items, lanes, versions, identities and merges. It does
//! not know photos, layers or clips. Apps plug in through `connector::Connector`.
#![forbid(unsafe_code)]

pub mod conformance;
pub mod connector;
pub mod identity;
pub mod jsonmerge;
pub mod merge;
pub mod project;
pub mod repo;
pub mod seal;
pub mod store;
pub mod tree;
pub mod wire;
pub mod workflow;
