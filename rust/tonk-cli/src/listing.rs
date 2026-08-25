//! One shape for every listing the CLI prints.
//!
//! `concept ls`, `view ls`, `blob ls`, `remote list`, `space list`,
//! `account spaces` and `account devices` each used to roll their own:
//! some carried a header row, most did not; some printed a parenthesised
//! line when there was nothing to show, most printed nothing at all; an
//! absent value was `-` in three of them and an empty cell in the rest.
//! Seven listings meant seven formats to learn.
//!
//! The one format here is `space list`'s, the newest of them and the only
//! one that was designed rather than accumulated: a header row, tab
//! separated cells, [`ABSENT`] where a value is missing, and a
//! parenthesised sentence when there are no rows at all. Tabs rather than
//! aligned columns because `cut -f` is the thing scripts reach for; the
//! stable machine-readable form is `--json`, which every one of these
//! verbs now takes; each serializes its own row type there, so a boolean
//! stays a boolean and an absent value is `null` rather than [`ABSENT`].

/// Printed for a cell whose value is absent.
///
/// A visible placeholder rather than an empty cell: an empty one collapses
/// against its neighbours under `cut -f` and turns a missing value into a
/// shifted row.
pub const ABSENT: &str = "-";

/// A listing on its way to stdout.
pub struct Listing {
    columns: &'static [&'static str],
    rows: Vec<Vec<String>>,
    empty: String,
    notes: Vec<String>,
}

impl Listing {
    /// A listing over `columns`, with `empty` as the whole output when no
    /// row is ever pushed. Write `empty` as a sentence that says what is
    /// missing and how to get one — it is the only thing a reader who ran
    /// the command too early will see.
    pub fn new(columns: &'static [&'static str], empty: impl Into<String>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            empty: empty.into(),
            notes: Vec::new(),
        }
    }

    /// Append one row. Cell count must match the column count.
    pub fn push<I, S>(&mut self, cells: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let row: Vec<String> = cells
            .into_iter()
            .map(Into::into)
            .map(|cell: String| escape_cell(&cell))
            .collect();
        debug_assert_eq!(
            row.len(),
            self.columns.len(),
            "listing row {row:?} does not match columns {:?}",
            self.columns
        );
        self.rows.push(row);
    }

    /// Add a paragraph printed after the rows — for the one thing a
    /// listing sometimes has to explain about what it just showed.
    /// Suppressed along with everything else when there are no rows.
    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// The listing as it goes to stdout, without a trailing newline.
    pub fn render(&self) -> String {
        if self.rows.is_empty() {
            return format!("({})", self.empty);
        }
        let mut out = self.columns.join("\t");
        for row in &self.rows {
            out.push('\n');
            out.push_str(&row.join("\t"));
        }
        for note in &self.notes {
            out.push_str("\n\n");
            out.push_str(note);
        }
        out
    }
}

/// A cell for a value that may be absent.
pub fn cell(value: Option<&str>) -> String {
    value.unwrap_or(ABSENT).to_owned()
}

/// Keep one logical value inside one physical TSV cell.
fn escape_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_renders_a_header_then_tab_separated_rows() {
        let mut listing = Listing::new(&["NAME", "KIND"], "nothing here");
        listing.push(["a", "first"]);
        listing.push(["b", "second"]);
        assert_eq!(listing.render(), "NAME\tKIND\na\tfirst\nb\tsecond");
    }

    #[dialog_common::test]
    fn it_replaces_the_whole_output_when_there_are_no_rows() {
        let listing = Listing::new(
            &["NAME"],
            "no spaces registered; make one with `tonk space new`",
        );
        assert_eq!(
            listing.render(),
            "(no spaces registered; make one with `tonk space new`)"
        );
    }

    /// A note explains the rows, so with no rows there is nothing for it
    /// to explain and printing it would contradict the empty line above.
    #[dialog_common::test]
    fn it_drops_a_note_when_there_are_no_rows() {
        let mut listing = Listing::new(&["NAME"], "nothing here");
        listing.note("some of these are special");
        assert_eq!(listing.render(), "(nothing here)");
    }

    #[dialog_common::test]
    fn it_prints_a_note_under_the_rows() {
        let mut listing = Listing::new(&["NAME"], "nothing here");
        listing.push(["a"]);
        listing.note("some of these are special");
        assert_eq!(listing.render(), "NAME\na\n\nsome of these are special");
    }

    #[dialog_common::test]
    fn it_marks_an_absent_cell_rather_than_leaving_it_blank() {
        let mut listing = Listing::new(&["NAME", "ACCOUNT"], "nothing here");
        listing.push([cell(Some("garden")), cell(None)]);
        assert_eq!(listing.render(), "NAME\tACCOUNT\ngarden\t-");
    }

    #[dialog_common::test]
    fn it_escapes_control_characters_inside_cells() {
        let mut listing = Listing::new(&["NAME", "DID"], "nothing here");
        listing.push(["work\tstation\nupstairs", "did:key:device"]);
        assert_eq!(
            listing.render(),
            "NAME\tDID\nwork\\tstation\\nupstairs\tdid:key:device"
        );
    }
}
