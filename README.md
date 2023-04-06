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
A search query is a a string consisting of two parts separated by a colon.
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

#### Caveat

There are a couple of assumptions currently being made that are linguistically
enforced by SQL itself, which means that there are a handful of forms that
this this program will currently not find.

1) Statements do not contain empty lines.\
   The following statement would not be matched:
   ```sql
    UPDATE ord
    SET status = 'CANCELLED'

    FROM orders ord
    WHERE ord.id = 1;
   ```
2)  **UPDATE:** Support for this form has been added in v0.1.1.\
    ~~UPDATE and DELETE statements targeting tables directly after the
    FROM clause (and not on any of the joined tables).~~
    ```
    UPDATE customers SET free_shipping = 1 WHERE id = @my_id
    ```
3)  **UPDATE:** Support for this form has been added in v0.1.1.\
    ~~Updates where the target table is not the table associated with the FROM
    keyword.~~\
    e.g. Given the following search query: `u:orders`:
    ```sql
    -- this would be matched
    UPDATE ord
    SET status = 'CANCELLED'
    FROM orders ord
      INNER JOIN customers cst
        ON	ord.customer_id = cst.id
    WHERE ord.id = 1;

    -- this would *not* be matched
    UPDATE ord
    SET status = 'CANCELLED'
    FROM customers cst
      INNER JOIN orders ord
        ON	cst.id = ord.customer_id
    WHERE cst.id = 1;
    ```
    ~~

Matching such statements is on the roadmap, because the author's coding style
preferences are not to be held above making this program as robust as possible :)\
If the (T)SQL is valid*, this program aims to support it.

    *The author reserves the right to hold-off on supporting implicit
    joins until a particularly rainy day...
