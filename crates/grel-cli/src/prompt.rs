//! Interactive prompt utilities.

use std::io::{self, Write};

/// Ask a yes/no question and return the answer
pub fn ask_confirmation(prompt: &str, default_yes: bool) -> bool {
    let default_str = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{prompt} {default_str}: ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    let input = input.trim().to_lowercase();
    match input.as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        "" => default_yes,
        _ => false,
    }
}

/// Display a warning message
pub fn warn(message: &str) {
    eprintln!("⚠️  {message}");
}

/// Display an info message
pub fn info(message: &str) {
    eprintln!("ℹ️  {message}");
}

/// Display an error message
pub fn error(message: &str) {
    eprintln!("❌ {message}");
}

/// Display a success message
pub fn success(message: &str) {
    eprintln!("✅ {message}");
}

/// Display a tip message
pub fn tip(message: &str) {
    eprintln!("💡 {message}");
}

/// Prompt user to select from a numbered list
pub fn select_from_list(prompt: &str, options: &[String]) -> Option<usize> {
    println!("{prompt}");
    for (i, option) in options.iter().enumerate() {
        println!("  {}) {}", i + 1, option);
    }
    print!("Select [1-{}]: ", options.len());
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    input.trim().parse::<usize>().ok().filter(|&n| n > 0 && n <= options.len())
}

/// Check if running in interactive terminal
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}
