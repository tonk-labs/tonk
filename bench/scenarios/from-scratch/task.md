You are working in a directory that is a tonk site, already connected
to its remote. Build a small habit tracker in the tonk system from
scratch using the tonk CLI (run `tonk guide` to learn it).

Requirements:

1. A concept named `habit` with a name and a daily target description.
2. A concept named `entry` recording one completion of a habit on a
   date (string date is fine).
3. Seed data: three habits ("Morning run", "Read 20 pages", "Inbox
   zero") and at least four entries across them spanning two dates.
4. A view named `habits` showing each habit with its name and target.
5. A view named `log` showing entries with habit name and date.

Stop when `tonk status` reports the branch is synced.
