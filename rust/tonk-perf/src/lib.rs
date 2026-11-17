//! Tonk's perf harness, as a library: the shaped network link a test or a
//! run puts between a client and the services it reaches.
//!
//! The `tonk-perf` binary (feature `cli`) adds the logging front proxy and
//! the run analyzer `bench/perf` uses, and runs links from a script.

pub mod link;
