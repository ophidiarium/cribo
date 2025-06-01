use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Python 3.10+ standard library modules
/// This list includes the most common standard library modules
/// A lazily-initialized static set of Python standard library module names
static STD_LIB_MODULES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut modules = HashSet::new();
    // Core modules
    modules.insert("sys");
    modules.insert("os");
    modules.insert("io");
    modules.insert("re");
    modules.insert("json");
    modules.insert("time");
    modules.insert("datetime");
    modules.insert("collections");
    modules.insert("itertools");
    modules.insert("functools");
    modules.insert("operator");
    modules.insert("math");
    modules.insert("random");
    modules.insert("string");
    modules.insert("pathlib");
    modules.insert("typing");
    modules.insert("abc");
    modules.insert("copy");
    modules.insert("pickle");
    modules.insert("logging");
    modules.insert("warnings");
    modules.insert("contextlib");
    modules.insert("argparse");
    modules.insert("subprocess");
    modules.insert("threading");
    modules.insert("multiprocessing");
    modules.insert("queue");
    modules.insert("asyncio");
    modules.insert("concurrent");
    // File and data formats
    modules.insert("csv");
    modules.insert("xml");
    modules.insert("html");
    modules.insert("email");
    modules.insert("base64");
    modules.insert("binascii");
    modules.insert("struct");
    modules.insert("codecs");
    // Network and internet
    modules.insert("urllib");
    modules.insert("http");
    modules.insert("socket");
    modules.insert("ssl");
    modules.insert("ftplib");
    modules.insert("smtplib");
    // Compression
    modules.insert("gzip");
    modules.insert("zipfile");
    modules.insert("tarfile");
    modules.insert("zlib");
    // Testing
    modules.insert("unittest");
    modules.insert("doctest");
    // Development tools
    modules.insert("pdb");
    modules.insert("profile");
    modules.insert("pstats");
    modules.insert("timeit");
    modules.insert("trace");
    modules.insert("traceback");
    modules.insert("inspect");
    modules.insert("ast");
    modules.insert("dis");
    modules.insert("code");
    modules.insert("codeop");
    // Platform specific
    modules.insert("platform");
    modules.insert("ctypes");
    modules.insert("mmap");
    modules.insert("select");
    modules.insert("fcntl");
    modules.insert("termios");
    // Misc
    modules.insert("hashlib");
    modules.insert("hmac");
    modules.insert("secrets");
    modules.insert("uuid");
    modules.insert("enum");
    modules.insert("dataclasses");
    modules.insert("importlib");
    modules.insert("pkgutil");
    modules.insert("modulefinder");
    modules.insert("runpy");
    modules.insert("gc");
    modules.insert("weakref");
    modules.insert("types");
    modules.insert("builtins");
    modules
});

/// Return the lazily-initialized set of standard library module names
pub fn get_stdlib_modules() -> &'static HashSet<&'static str> {
    &STD_LIB_MODULES
}

pub fn is_stdlib_module(module_name: &str) -> bool {
    // Check direct match
    if STD_LIB_MODULES.contains(module_name) {
        return true;
    }
    // Check if it's a submodule of a stdlib module
    if let Some(top_level) = module_name.split('.').next() {
        STD_LIB_MODULES.contains(top_level)
    } else {
        false
    }
}
