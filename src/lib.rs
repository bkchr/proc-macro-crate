/*!

[![](https://docs.rs/proc-macro-crate/badge.svg)](https://docs.rs/proc-macro-crate/) [![](https://img.shields.io/crates/v/proc-macro-crate.svg)](https://crates.io/crates/proc-macro-crate) [![](https://img.shields.io/crates/d/proc-macro-crate.png)](https://crates.io/crates/proc-macro-crate) [![Build Status](https://travis-ci.org/bkchr/proc-macro-crate.png?branch=master)](https://travis-ci.org/bkchr/proc-macro-crate)

Providing support for `$crate` in procedural macros.

* [Introduction](#introduction)
* [Example](#example)
* [License](#license)

## Introduction

In `macro_rules!` `$crate` is used to get the path of the crate where a macro is declared in. In
procedural macros there is currently no easy way to get this path. A common hack is to import the
desired crate with a know name and use this. However, with rust edition 2018 and dropping
`extern crate` declarations from `lib.rs`, people start to rename crates in `Cargo.toml` directly.
However, this breaks importing the crate, as the proc-macro developer does not know the renamed
name of the crate that should be imported.

This crate provides a way to get the name of a crate, even if it renamed in `Cargo.toml`. For this
purpose a single function `crate_name` is provided. This function needs to be called in the context
of a proc-macro with the name of the desired crate. `CARGO_MANIFEST_DIR` will be used to find the
current active `Cargo.toml` and this `Cargo.toml` is searched for the desired crate.

## Example

```
use quote::quote;
use syn::Ident;
use proc_macro2::Span;
use proc_macro_crate::{crate_name, FoundCrate};

fn import_my_crate() {
    let found_crate = crate_name("my-crate").expect("my-crate is present in `Cargo.toml`");

    match found_crate {
        FoundCrate::Itself => quote!( crate::Something ),
        FoundCrate::Name(name) => {
            let ident = Ident::new(&name, Span::call_site());
            quote!( #ident::Something )
        }
    };
}

# fn main() {}
```

## License

Licensed under either of

 * [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)

 * [MIT license](http://opensource.org/licenses/MIT)

at your option.
*/

use std::{
    collections::BTreeMap,
    env,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use once_cell::sync::OnceCell;
use toml::{self, value::Table};

/// Error type used by this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Could not find `Cargo.toml` in manifest dir: `{0}`.")]
    NotFound(PathBuf),
    #[error("`CARGO_MANIFEST_DIR` env variable not set.")]
    CargoManifestDirNotSet,
    #[error("Could not read `{path}`.")]
    CouldNotRead { path: PathBuf, source: io::Error },
    #[error("Invalid toml file.")]
    InvalidToml { source: toml::de::Error },
    #[error("Could not find `{crate_name}` in `dependencies` or `dev-dependencies` in `{path}`!")]
    CrateNotFound { crate_name: String, path: PathBuf },
}

/// The crate as found by [`crate_name`].
#[derive(Debug, PartialEq, Clone, Eq)]
pub enum FoundCrate {
    /// The searched crate is this crate itself.
    Itself,
    /// The searched crate was found with this name.
    Name(String),
}

/// Find the crate name for the given `orig_name` in the current `Cargo.toml`.
///
/// `orig_name` should be the original name of the searched crate.
///
/// The current `Cargo.toml` is determined by taking `CARGO_MANIFEST_DIR/Cargo.toml`.
///
/// # Returns
///
/// - `Ok(orig_name)` if the crate was found, but not renamed in the `Cargo.toml`.
/// - `Ok(RENAMED)` if the crate was found, but is renamed in the `Cargo.toml`. `RENAMED` will be
/// the renamed name.
/// - `Err` if an error occurred.
///
/// The returned crate name is sanitized in such a way that it is a valid rust identifier. Thus,
/// it is ready to be used in `extern crate` as identifier.
pub fn crate_name(orig_name: &str) -> Result<FoundCrate, Error> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(|_| Error::CargoManifestDirNotSet)?;

    struct Cache {
        manifest_dir: String,
        manifest_path: PathBuf,
        crate_names: BTreeMap<String, FoundCrate>,
    }

    static CACHE: OnceCell<Cache> = OnceCell::new();
    let cache = CACHE.get_or_try_init(|| {
        let manifest_dir = manifest_dir.clone();
        let manifest_path = Path::new(&manifest_dir).join("Cargo.toml");

        if !manifest_path.exists() {
            return Err(Error::NotFound(manifest_dir.into()));
        }

        let manifest = open_cargo_toml(&manifest_path)?;
        let crate_names = extract_crate_names(&manifest)?;

        Ok(Cache {
            manifest_dir,
            manifest_path,
            crate_names,
        })
    })?;

    assert_eq!(
        manifest_dir, cache.manifest_dir,
        "CARGO_MANIFEST_DIR must not change within one compiler process"
    );

    Ok(cache
        .crate_names
        .get(orig_name)
        .ok_or_else(|| Error::CrateNotFound {
            crate_name: orig_name.to_owned(),
            path: cache.manifest_path.clone(),
        })?
        .clone())
}

/// Make sure that the given crate name is a valid rust identifier.
fn sanitize_crate_name<S: AsRef<str>>(name: S) -> String {
    name.as_ref().replace('-', "_")
}

/// Open the given `Cargo.toml` and parse it into a hashmap.
fn open_cargo_toml(path: &Path) -> Result<Table, Error> {
    let mut content = String::new();
    File::open(path)
        .map_err(|e| Error::CouldNotRead {
            source: e,
            path: path.into(),
        })?
        .read_to_string(&mut content)
        .map_err(|e| Error::CouldNotRead {
            source: e,
            path: path.into(),
        })?;
    toml::from_str(&content).map_err(|e| Error::InvalidToml { source: e })
}

/// Extract all crate names from the given `Cargo.toml` by checking the `dependencies` and
/// `dev-dependencies`.
fn extract_crate_names(cargo_toml: &Table) -> Result<BTreeMap<String, FoundCrate>, Error> {
    let package_name = extract_package_name(cargo_toml);
    let root_pkg = package_name.map(|name| {
        let cr = match env::var_os("CARGO_TARGET_TMPDIR") {
            // We're running for a library/binary crate
            None => FoundCrate::Itself,
            // We're running for an integration test
            Some(_) => FoundCrate::Name(sanitize_crate_name(name)),
        };

        (name.to_owned(), cr)
    });

    let dep_tables = dep_tables(cargo_toml).chain(target_dep_tables(cargo_toml));
    let dep_pkgs = dep_tables.flatten().map(|(dep_name, dep_value)| {
        let pkg_name = dep_value
            .as_table()
            .and_then(|t| t.get("package")?.as_str())
            .unwrap_or(dep_name);
        let cr = FoundCrate::Name(sanitize_crate_name(dep_name));

        (pkg_name.to_owned(), cr)
    });

    Ok(root_pkg.into_iter().chain(dep_pkgs).collect())
}

fn extract_package_name(cargo_toml: &Table) -> Option<&str> {
    cargo_toml.get("package")?.as_table()?.get("name")?.as_str()
}

fn target_dep_tables(cargo_toml: &Table) -> impl Iterator<Item = &Table> {
    cargo_toml
        .get("target")
        .into_iter()
        .filter_map(toml::Value::as_table)
        .flat_map(|t| {
            t.values()
                .filter_map(toml::Value::as_table)
                .flat_map(dep_tables)
        })
}

fn dep_tables(table: &Table) -> impl Iterator<Item = &Table> {
    table
        .get("dependencies")
        .into_iter()
        .chain(table.get("dev-dependencies"))
        .filter_map(toml::Value::as_table)
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! create_test {
        (
            $name:ident,
            $cargo_toml:expr,
            $( $result:tt )*
        ) => {
            #[test]
            fn $name() {
                let cargo_toml = toml::from_str($cargo_toml).expect("Parses `Cargo.toml`");

                match extract_crate_names(&cargo_toml).map(|mut map| map.remove("my_crate")) {
                    $( $result )* => (),
                    o => panic!("Invalid result: {:?}", o),
                }
            }
        };
    }

    create_test! {
        deps_with_crate,
        r#"
            [dependencies]
            my_crate = "0.1"
        "#,
        Ok(Some(FoundCrate::Name(name))) if name == "my_crate"
    }

    create_test! {
        dev_deps_with_crate,
        r#"
            [dev-dependencies]
            my_crate = "0.1"
        "#,
        Ok(Some(FoundCrate::Name(name))) if name == "my_crate"
    }

    create_test! {
        deps_with_crate_renamed,
        r#"
            [dependencies]
            cool = { package = "my_crate", version = "0.1" }
        "#,
        Ok(Some(FoundCrate::Name(name))) if name == "cool"
    }

    create_test! {
        deps_with_crate_renamed_second,
        r#"
            [dependencies.cool]
            package = "my_crate"
            version = "0.1"
        "#,
        Ok(Some(FoundCrate::Name(name))) if name == "cool"
    }

    create_test! {
        deps_empty,
        r#"
            [dependencies]
        "#,
        Ok(None)
    }

    create_test! {
        crate_not_found,
        r#"
            [dependencies]
            serde = "1.0"
        "#,
        Ok(None)
    }

    create_test! {
        target_dependency,
        r#"
            [target.'cfg(target_os="android")'.dependencies]
            my_crate = "0.1"
        "#,
        Ok(Some(FoundCrate::Name(name))) if name == "my_crate"
    }

    create_test! {
        target_dependency2,
        r#"
            [target.x86_64-pc-windows-gnu.dependencies]
            my_crate = "0.1"
        "#,
        Ok(Some(FoundCrate::Name(name))) if name == "my_crate"
    }

    create_test! {
        own_crate,
        r#"
            [package]
            name = "my_crate"
        "#,
        Ok(Some(FoundCrate::Itself))
    }
}
