/* gresql - grep sql statements
 * ----------------------------------------------------------------------------
 * summary:
 *   script for searching for stored procedures that contain specific statements
 *   instead of simplyng matching strings in the file (like grep),
 *   this script will all you to specify the type of statement and the table(s)
 *   it is referencing.
 * ----------------------------------------------------------------------------
 * usage:
 *   gresql -s <typestr>:<table>,... [<path|file> ...]
 *   where <type> is a string of one or more of the following characters:
 *      s (select), i (insert), u (update), d (delete), m (merge)
 *   and <table> is comma-separated list of tables to search
 *
 *   e.g.
 *     `gresql -s ud:t_pick_detail`
 *     will search for updates or deletes to t_pick_detail in the current directory
 *
 *     `gresql -s u:t_order,t_order_detail ./usp_wave_mgmt*.sql`
 *     will search for updates to t_order or t_order_detail in all
 *     wave management sprocs in the current directory
 *
 *     multiple -s options can be specified and will accumulate as an `AND` search
 *     `gresql -s u:t_pick_detail -s d:t_pick_detail`
 *     will search for sprocs that have both updates AND deletes to t_pick_detail
 */
#![feature(buf_read_has_data_left)]
#![feature(hash_drain_filter)]

extern crate exitcode;

use clap::Parser;
use glob::glob;
use regex::Regex;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::File;
use std::io::{ BufRead, BufReader, Write };
use std::path::{ Path, PathBuf };
use grep_regex::RegexMatcher;
use grep_searcher::Searcher;
use grep_searcher::sinks::Bytes;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[arg(short = 's', long = "search", required = true, help = "Search query")]
    search_queries: Vec<String>,
    #[arg(short = 'd', long = "delimiter", default_value_t=',', help = "Result field-delimiter")]
    delimiter: char,
    // boolean flags
    #[arg(short = 'p', long = "path-only", default_value_t = false, help = "Only print the paths of matching files")]
    only_file_paths: bool,
    #[arg(short = 'T', long = "no-statement-text", default_value_t = false, help = "Don't print statement text")]
    hide_statement: bool,
    #[arg(short = 'v', long = "verbose", default_value_t = false, help = "Verbose output")]
    verbose: bool,
    // remaining arguments are file-paths
    #[arg(required = false, default_values_os_t = vec![OsString::from(".")], help = "File(s) to process")]
    file_paths: Vec<OsString>,
}

struct PrintOpts {
    only_file_paths: bool,
    hide_statement:  bool,
    delimiter:       char,
}

// statement types ============================================================
#[derive(Debug, PartialEq)]
pub enum StatementType {
    Select,
    Insert,
    Update,
    Delete,
    Merge,
}

// implement try_from &char for StatementType
impl TryFrom<char> for StatementType {
    type Error = ();
    fn try_from(c: char) -> Result<Self, Self::Error> {
        match c {
            's' => Ok(StatementType::Select),
            'i' => Ok(StatementType::Insert),
            'u' => Ok(StatementType::Update),
            'd' => Ok(StatementType::Delete),
            'm' => Ok(StatementType::Merge),
            _ => Err(()),
        }
    }
}

// implement try_from String for StatementType
impl TryFrom<String> for StatementType {
    type Error = ();
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "select" => Ok(StatementType::Select),
            "insert" => Ok(StatementType::Insert),
            "update" => Ok(StatementType::Update),
            "delete" => Ok(StatementType::Delete),
            "merge"  => Ok(StatementType::Merge),
            _ => Err(()),
        }
    }
}

// display for StatementType
impl std::fmt::Display for StatementType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatementType::Select => write!(f, "SELECT"),
            StatementType::Insert => write!(f, "INSERT"),
            StatementType::Update => write!(f, "UPDATE"),
            StatementType::Delete => write!(f, "DELETE"),
            StatementType::Merge  => write!(f, "MERGE"),
        }
    }
}

fn parse_statement_types(statement_types: &str) -> Vec<StatementType> {
    // get set of unique characters in statement_types
    let char_set: HashSet<char> = statement_types.chars().collect();
    let mut statement_types: Vec<StatementType> = Vec::new();
    if char_set.contains(&'*') {
        // return all statement types except select
        return vec![
            StatementType::Insert,
            StatementType::Update,
            StatementType::Delete,
            StatementType::Merge,
        ];
    }
    char_set.into_iter()
        .filter_map(|c| StatementType::try_from(c).ok())
        .for_each(|stmt_type| statement_types.push(stmt_type));
    statement_types
}
// ============================================================================

// search queries =============================================================
#[derive(Debug)]
struct SearchQuery {
    statement_types: Vec<StatementType>,
    tables: Vec<String>,
}

impl SearchQuery {
    fn statement_pattern(&self) -> String {
        let mut pattern = String::new();
        pattern.push_str(r"\b((?i)");
        pattern.push_str(
            &self.statement_types.iter()
                .map(|st| st.to_string())
                .collect::<Vec<String>>()
                .join("|"));
        pattern.push_str(r")\b");
        pattern
    }
    fn table_pattern(&self) -> String {
        let mut pattern = String::new();
        pattern.push_str(r"(");
        pattern.push_str(&self.tables.join("|"));
        pattern.push_str(r")\b");
        pattern
    }
}

fn parse_search_queries(strings: Vec<String>) -> Vec<SearchQuery> {
    strings.iter()
        .map(|s| { s.split(':').collect() })
        .filter_map(|ps: Vec<&str>| {
            match ps.len() {
                1 => Some(SearchQuery {
                    statement_types: parse_statement_types("*"),
                    tables: ps[0].split(",").map(String::from).collect(),
                }),
                2 => Some(SearchQuery {
                    statement_types: parse_statement_types(ps[0]),
                    tables: ps[1].split(",").map(String::from).collect(),
                }),
                _ => None
            }})
        .collect()
}
// ============================================================================

// file paths =================================================================
pub enum PathType {
    File,
    Directory,
    Symlink,
}

fn get_path_type(path: &Path) -> Option<PathType> {
    if !path.exists() { return None; }
    match path.is_file() {
        true  => Some(PathType::File),
        false => match path.is_dir() {
            true  => Some(PathType::Directory),
            false => Some(PathType::Symlink),
        },
    }
}

// TODO: this appeared to run super slow, investigate
fn get_file_paths(strings: &Vec<OsString>) -> HashSet<PathBuf> {
    // return a vector of resolved path buffers from a vector of strings, of
    // which each string could be a file, a symlink, a directory, or a glob
    // pattern
    let mut paths: HashSet<PathBuf> = HashSet::new();
    for s in strings {
        let path: &Path = Path::new(s);
        if let Some(path_type) = get_path_type(path) { // valid path
            match path_type {
                PathType::File => { paths.insert(PathBuf::from(s)); },
                PathType::Symlink => {
                    if let Ok(link_path) = path.read_link() {
                        paths.insert(PathBuf::from(link_path.to_str().unwrap()));
                    }
                }
                PathType::Directory => {
                    // get all files in directory
                    let mut dir_path = PathBuf::from(s);
                    dir_path.push("**/*.sql");
                    for entry in glob(dir_path.to_str().unwrap()).unwrap() {
                        if let Ok(entry) = entry {
                            paths.insert(entry);
                        }
                    }
                }
            }
        } else if let Some(_) = s.to_str().unwrap().find('*') { // glob pattern
            for entry in glob(s.to_str().unwrap()).unwrap() {
                if let Ok(entry) = entry {
                    paths.insert(entry);
                }
            }
        } else {
            eprintln!("File not found: {}", s.to_str().unwrap());
        }
    }
    paths
}
// ============================================================================
#[derive(Debug)]
struct Statement {
    file_path:      PathBuf,
    statement_type: StatementType,
    table:          String,
    begin:          usize,
    end:            usize,
    text:           String,
}

fn find_statements(file_path: &PathBuf, search_query: &SearchQuery) -> Option<Vec<Statement>> {
    // return a vector of all the statements from a file that match the search query
    // TODO: add support for statements that begin with CTEs
    let file = File::open(file_path).unwrap();
    let mut reader = BufReader::new(file);
    let mut statements = Vec::<Statement>::new();

    let read_next_line = |reader: &mut BufReader<File>| -> String {
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        line
    };

    let try_statement_type_from_line = |line: String| -> Option<StatementType> {
        let first_word: String = line.split_whitespace().next().unwrap().to_lowercase();
        if let Ok(statement_type) = StatementType::try_from(first_word) {
            return Some(statement_type);
        }
        None
    };

    let parse_table = |statement_type: &StatementType, s: &str| -> Option<String> {
        let re = match statement_type {
            StatementType::Insert => {
                Regex::new(r"\b(?i:into)\s+([@#[:alnum:]_]+)").expect("regex didn't compile")
            },
            StatementType::Select | StatementType::Update | StatementType::Delete => {
                Regex::new(r"\b(?i:from)\s+([@#[:alnum:]_]+)").expect("regex didn't compile")
            },
            StatementType::Merge => {
                Regex::new(r"\b(?i:merge)\s+([@#[:alnum:]_]+)").expect("regex didn't compile")
            },
        };

        if let Some(capts) = re.captures(&s) {
            match capts.get(1) {
                Some(table) => Some(table.as_str().to_string()),
                None => None
            }
        } else {
            // TODO: I'm sure there's a way to reduce the code duplication here
            match statement_type {
                StatementType::Update => {
                    let re2 = Regex::new(r"\b(?i:update)\s+([@#[:alnum:]_]+)").expect("regex didn't compile");
                    match re2.captures(&s) {
                        Some(capts) => Some(capts.get(1).unwrap().as_str().to_string()),
                        None => None
                    }
                },
                StatementType::Delete => {
                    let re2 = Regex::new(r"\b(?i:delete)\s+([@#[:alnum:]_]+)").expect("regex didn't compile");
                    match re2.captures(&s) {
                        Some(capts) => Some(capts.get(1).unwrap().as_str().to_string()),
                        None => None
                    }
                },
                _ => None
            }
        }
    };

    let trim_comment = |s: String| -> String {
        match s.find("--") {
            Some(i) => s[..i].to_string(),
            None => s
        }
    };

    let clean_text = |s: String| -> String { trim_comment(s.replace("\t", " ")) };

    let mut comment_level: u8 = 0;
    let mut i: isize = -1;
    // while let Ok(line) = reader.read_line().unwrap().trim().trim_start_matches(';').to_string() {
    while let Ok(is_more_to_read) = reader.has_data_left() {
        if !is_more_to_read { break; }
        i+= 1;
        let line = read_next_line(&mut reader)
            .trim()
            .trim_start_matches(';')
            .to_string();

        if line.is_empty()        { continue; }
        if line.starts_with("--") { continue; }
        if line.contains("/*")    { comment_level +=1; }
        if line.contains("*/")    { comment_level -=1; }
        if comment_level > 0      { continue; }

        // check if the first word of the line is the start of a statement that
        // we care about based on the search query
        if let Some(statement_type) = try_statement_type_from_line(line.clone()) {
            if !search_query.statement_types.contains(&statement_type) { continue; }
            // if we're in a statement type that was in the search query, then
            // we need to read the entire query to determine whether contains
            // a table from the search query
            // TODO: this can maybe be optimized by checking each line to see
            // if it has one of the keywords preceeding the table name, adding
            // the following line to statement_text if it does, and then
            // checking statement_text for the table.
            let begin: usize = i.try_into().expect("i should be positive by the time the loop starts");
            // let mut statement_text = line.to_string() + " ";
            let mut statement_text = clean_text(line) + " ";
            while let Ok(is_more_to_read) = reader.has_data_left() {
                i += 1;
                let line = read_next_line(&mut reader)
                    .trim()
                    .to_string();

                if line.starts_with("--") { continue; }
                if line.contains("/*")    { comment_level +=1; }
                if line.contains("*/")    { comment_level -=1; }
                if comment_level > 0      { continue; }

                // start building up statement_text by concatenating each line
                // until we reach an empty line or a semi-colon, which signals
                // the end of the statement
                if !line.is_empty() && !line.starts_with(";") {
                    statement_text.push_str(&(clean_text(line) + " "));
                    if is_more_to_read { continue; }
                }

                if let Some(table) = parse_table(&statement_type, &statement_text) {
                    if search_query.tables.contains(&table) {
                        statements.push(Statement {
                            file_path:      file_path.to_path_buf(),
                            statement_type: statement_type,
                            table:          table,
                            begin:          begin,
                            end:            i.try_into().expect("i should be positive by the time the loop starts"),
                            text:           statement_text,
                        });
                    }
                }
                break;
            }
        }
    }
    match statements.len() {
        0 => None,
        _ => Some(statements),
    }
}

fn print_statements(opts: PrintOpts, statements: Vec<Statement>) {
    let del: char = opts.delimiter;
    let stdout    = std::io::stdout();
    let mut lock  = stdout.lock();

    if opts.hide_statement {
        for s in statements {
            writeln!(lock, "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
                s.file_path.display(), del,
                s.begin, del,
                s.end, del,
                s.statement_type, del,
                s.table
                ).unwrap();
        }
        return;
    }

    for s in statements {
        writeln!(lock, "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
            s.file_path.display(), del,
            s.begin, del,
            s.end, del,
            s.statement_type, del,
            s.table, del,
            s.text
            ).unwrap();
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn parse_statement_types() {
        let invalid_characters: Vec<char> = vec![';', 'a', '\n', '*'];
        for c in invalid_characters {
            assert!(super::StatementType::try_from(c).is_err());
        }

        let invalid_strings: Vec<&str> = vec!["alter", "declare", "apply", "grant", ""];
        for s in invalid_strings {
            assert!(super::StatementType::try_from(s.to_string()).is_err());
        }
    }

}

fn main() {
    let cli = Cli::parse();

    let search_queries: Vec<SearchQuery> = parse_search_queries(cli.search_queries);
    let file_paths: HashSet<PathBuf> = get_file_paths(&cli.file_paths);
    let print_opts: PrintOpts = PrintOpts {
        only_file_paths: cli.only_file_paths,
        hide_statement:  cli.hide_statement,
        delimiter:       cli.delimiter,
    };

    if cli.verbose {
        dbg!(&search_queries);

        println!("Paths given:");
        dbg!(&cli.file_paths);

        println!("Paths found:");
        dbg!(&file_paths);
    }

    // first step is to do a basic search for all the files that contain the
    // tables and the statement types.
    // this search is only the first step to narrow-down the file-list.
    // e.g. it won't tell us if a file has an update statement to `orders`, only
    // that a file contains both an update statement and `orders`.
    let mut matched_files: HashSet<PathBuf> = HashSet::new();
    let mut searcher = Searcher::new();
    for path in &file_paths {
        let file_is_match = |search_query: &SearchQuery| -> bool {
            for pattern in vec![&search_query.statement_pattern(), &search_query.table_pattern()] {
                let matcher = RegexMatcher::new(pattern.as_str()).unwrap();
                let mut is_match = false;
                let set_found = |_l: u64, _s: &[u8]| -> Result<bool, _> {
                    is_match = true;
                    Ok(false) // return false to stop the search
                };

                if let Err(_) = searcher.search_path(&matcher, path, Bytes(set_found)) {
                    eprintln!("Error when searching {} for {}", path.display(), pattern);
                    return false;
                }

                // exit early if we didn't find a match
                if !is_match { return false; }
            }
            true
        };

        if search_queries.iter().map(file_is_match).all(|b| b) {
            matched_files.insert(path.clone());
        }
    }

    if cli.verbose {
        println!("STEP 1 RESULTS: {} files matched", matched_files.len());
        dbg!(&matched_files);
    }

    // build list of matching statements
    // if no matching statements are found in a given file, remove it from
    // matched_files
    let mut statements: Vec<Statement> = Vec::new();
    for query in search_queries.iter() {
        matched_files.drain_filter(|file_path| {
            if let Some(found_statements) = find_statements(file_path, query) {
                statements.extend(found_statements);
                true
            } else {
                false
            }
        });
    }

    if cli.verbose {
        println!("STEP 2 RESULTS: {} files matched", matched_files.len());
        dbg!(&matched_files);
    }

    if statements.is_empty() {
        eprintln!("No statements found");
        return;
    }

    if print_opts.only_file_paths {
        let stdout   = std::io::stdout();
        let mut lock = stdout.lock();
        for f in matched_files.iter() {
            writeln!(lock, "{:?}", f.display()).unwrap();
        }
        return;
    }

    print_statements(print_opts, statements);
}
