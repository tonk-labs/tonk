// The wire query for the language a block names by `word`: every record
// carrying that word, with its pack.
(self, word) => ({
  predicate: {
    with: {
      word: { the: 'xyz.tonk.code-editor.language/word', as: 'Text', cardinality: 'many' },
      source: { the: 'xyz.tonk.code-editor.language/source', as: 'Text', cardinality: 'one' },
    },
  },
  terms: {
    this: { '?': { name: 'this' } },
    word,
    source: { '?': { name: 'source' } },
  },
})
