//! Chain builder handles — [`RepositoryHandle`] and
//! [`BranchHandle`]. Both borrow names from the caller; nothing
//! is allocated by the chain itself.

use dialog_query::ConceptQuery;

use super::TonkReactor;
use super::query::Query;
use super::subscribe::Subscribe;

/// Builder — names the repository the chain operates on.
pub struct RepositoryHandle<'a> {
    reactor: &'a TonkReactor,
    repo: &'a str,
}

impl<'a> RepositoryHandle<'a> {
    pub(super) fn new(reactor: &'a TonkReactor, repo: &'a str) -> Self {
        Self { reactor, repo }
    }

    /// Narrow the chain to a specific branch.
    pub fn branch(self, name: &'a str) -> BranchHandle<'a> {
        BranchHandle {
            reactor: self.reactor,
            repo: self.repo,
            branch: name,
        }
    }
}

/// Builder — names a branch within a repository.
pub struct BranchHandle<'a> {
    reactor: &'a TonkReactor,
    repo: &'a str,
    branch: &'a str,
}

impl<'a> BranchHandle<'a> {
    /// One-shot read.
    pub fn query(self, query: ConceptQuery) -> Query<'a> {
        Query::new(self.reactor, self.repo, self.branch, query)
    }

    /// Open or attach to a standing subscription for `query`.
    pub fn subscribe(self, query: ConceptQuery) -> Subscribe<'a> {
        Subscribe::new(self.reactor, self.repo, self.branch, query)
    }
}
