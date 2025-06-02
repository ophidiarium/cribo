pub mod unparser;
mod utils;
pub use crate::unparser::Unparser;
#[cfg(test)]
mod comment_test;
#[cfg(feature = "transformer")]
pub mod transformer;
#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rustpython_ast::Fold;
    use rustpython_ast::TextSize;
    use rustpython_ast::text_size::TextRange;
    use rustpython_parser::Parse;
    use rustpython_parser::ast::Suite;

    use std::fs;
    use std::io;
    use std::path::Path;

    struct RangesEraser {}

    impl Fold<TextRange> for RangesEraser {
        type TargetU = TextRange;

        type Error = std::convert::Infallible;

        type UserContext = TextRange;

        fn will_map_user(&mut self, _user: &TextRange) -> Self::UserContext {
            TextRange::new(TextSize::new(0), TextSize::new(0))
        }

        fn map_user(
            &mut self,
            _user: TextRange,
            start: Self::UserContext,
        ) -> Result<Self::TargetU, Self::Error> {
            Ok(start)
        }
    }

    fn run_tests_on_folders(source_folder: &str, results_folder: &str) -> io::Result<()> {
        for entry in fs::read_dir(results_folder)? {
            let entry = entry?;

            let entry_path = entry.path();
            if entry_path.is_file()
                && entry_path.file_name().is_some_and(|name| {
                    name.to_str()
                        .is_some_and(|inner_name| inner_name.ends_with(".py"))
                })
            {
                fs::remove_file(entry_path)?;
            }
        }

        for entry in fs::read_dir(source_folder)? {
            let entry = entry?;

            let entry_path = entry.path();

            if entry_path.is_file()
                && entry_path.file_name().is_some_and(|name| {
                    name.to_str()
                        .is_some_and(|inner_name| inner_name.ends_with(".py"))
                })
            {
                let file_content = fs::read_to_string(&entry_path)?;
                let entry_path_str = entry_path.to_str().unwrap();
                let mut unparser = Unparser::new();
                let stmts = Suite::parse(&file_content, entry_path_str).unwrap();
                for stmt in &stmts {
                    unparser.unparse_stmt(stmt);
                }
                let new_source = unparser.source;
                let old_file_name = entry_path.file_name().unwrap().to_str().unwrap();
                let new_file_name = old_file_name.replace(".py", "_unparsed.py");
                let new_entry_path_str = format!("{}/{}", results_folder, new_file_name);
                let new_entry_path = Path::new(&new_entry_path_str);
                fs::write(new_entry_path, &new_source)?;
                let new_stmts =
                    Suite::parse(&new_source, new_entry_path.to_str().unwrap()).unwrap();
                // erase range information
                let mut eraser = RangesEraser {};
                let mut erased_new_stmts = Vec::new();
                for stmt in &new_stmts {
                    erased_new_stmts.push(eraser.fold_stmt(stmt.to_owned()).unwrap());
                }

                let mut erased_stmts = Vec::new();
                for stmt in &stmts {
                    erased_stmts.push(eraser.fold_stmt(stmt.to_owned()).unwrap());
                }

                for (stmt, new_stmt) in erased_stmts.iter().zip(erased_new_stmts.iter()) {
                    assert_eq!(stmt, new_stmt)
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_predefined_files() -> io::Result<()> {
        run_tests_on_folders("./test_files", "./test_files_unparsed")
    }
}
