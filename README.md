# GRESQL (Grey-Squirrel)

A command line interface for searching SQL files for specific statements

## Usage
```
gresql [OPTIONS] --search <SEARCH_QUERIES> [FILE_PATHS]...

Arguments:
  [FILE_PATHS]...  File(s) to process [default: .]

Options:
  -s, --search <SEARCH_QUERIES>  Search query
  -d, --delimiter <DELIMITER>    Result field-delimiter [default: ,]
  -p, --path-only                Only print the paths of matching files
  -T, --no-statement-text        Don't print statement text
  -v, --verbose                  Verbose output
  -h, --help                     Print help
  -V, --version                  Print version
```

If a directory is given in FILE_PATHS, then all .sql files in the directory
are processed.

### Search queries
A search queury is a a string consisting of two parts separated by a colon.
The first part is the statement type(s) represented by a single character.
The second part is the table(s) to search for (separated by commas).

Multiple statement-types or tables in a query will be treated as an OR search.
The `--search` option may be used multiple times, in which case a file must
match all of the search queries to be returned as a match.

Statement Types:
  - `d`: DELETE
  - `i`: INSERT
  - `m`: MERGE
  - `s`: SELECT
  - `u`: UPDATE

Example:\
  `gresql --search "u:orders" <file> ...`\
  search for update statements to the orders table

  `gresql --search "ud:orders,customers" <file> ...`\
  search for update statements or delete statements to to the orders table or
  the customers table

  `gresql --search "u:orders" --search "d:customers" <file> ...`\
  search for files containing both an update to the orders table and a delete
  to the customers table

  `gresql --search "orders"`\
  `gresql --search "*:orders"`\
  omitting the statement-type or specifying '*' from the search-query will
  search for all statement-types except `SELECT` (i.e. all statements that
  modify the given table).
