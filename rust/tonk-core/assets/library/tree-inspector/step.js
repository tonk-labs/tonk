// Ask one of dialog's named steps: a tree resolver (`tree/node`,
// `tree/span`, `tree/entry`, `tree/value`) or a formula
// (`dialog/key-part`, `dialog/separator-part`). `given` are the terms
// that are constants; each name in `wanted` becomes a variable of that
// name, and a row's fields are keyed by those names.
(self, name, given, wanted) => {
  const terms = { ...given };
  for (const slot of wanted) terms[slot] = { '?': { name: slot } };
  return self.query({ predicate: name, terms }).then((rows) => rows.map((row) => row?.fields ?? {}));
}
