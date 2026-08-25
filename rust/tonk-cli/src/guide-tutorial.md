# Tutorial: the loop

1. Discover what is present with `tonk show`, `tonk concept`, and `tonk view`.
2. Define a concept with `tonk concept add <name> --field
   <field>:<type>:<cardinality>`. The concept is immediately usable.
3. Create or update facts with `tonk assert`; read them with `tonk query`; use
   `tonk retract` to invalidate a field or instance.
4. Add a view with `tonk view add`. The first view automatically becomes the
   home when none is set; use `tonk space home` to replace or order the visible
   concept directories. Check the result headlessly with `tonk render`.
5. Use `tonk eval` for rules, effects, joins, or multi-statement documents the
   convenient verbs cannot express.
6. Share the space with `tonk invite` and let the recipient `tonk join` it.

Every notation-building write accepts `--notation` to print the document it
would evaluate. A committing write syncs automatically unless `--no-sync` is
set; `--dry-run` evaluates and plans but drops the transaction.
