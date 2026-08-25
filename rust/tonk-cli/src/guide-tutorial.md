# Tutorial: the loop

1. Discover what is present with `tonk show`, `tonk concept`, and `tonk view`.
2. Define a concept with `tonk concept add <name> --field
   <field>:<type>:<cardinality>`. The concept is immediately usable.
3. Create or update facts with `tonk assert`; read them with `tonk query`; use
   `tonk retract` to invalidate a field or instance.
4. Add a view with `tonk view add`. A first detail or directory view
   automatically becomes the home when none is set. Use `--home` to install a
   view and replace the home atomically, or `tonk space home` to repoint it
   later. Check the result headlessly with `tonk render`.
5. Use `tonk eval` for rules, effects, joins, or multi-statement documents the
   convenient verbs cannot express. On a raw first build,
   `tonk eval interactive.notation --home todo` installs the document and
   replaces the home in one transaction.
6. Share the space with `tonk invite` and let the recipient `tonk join` it.

Every notation-building write accepts `--notation` to print the document it
would evaluate. A committing write syncs automatically unless `--no-sync` is
set; `--dry-run` evaluates and plans but drops the transaction.

The shortest schema-to-visible-directory workflow is:

```text
tonk concept add todo --field title:text:one
tonk assert todo --title "Write the guide"
tonk view add todo --kind directory --template-file todo.html --home
tonk render todo
```

`--home` replaces the prior home with `todo`; it does not append to it.
