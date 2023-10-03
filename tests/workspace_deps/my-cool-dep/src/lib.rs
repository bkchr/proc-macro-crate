use proc_macro::TokenStream;

#[proc_macro]
pub fn do_something(input: TokenStream) -> TokenStream {
    let found_crate =
        proc_macro_crate::crate_name("my-cool-dep-real-name").expect("Couldn't find the crate");

    assert_eq!(proc_macro_crate::FoundCrate::Name("my_cool_dep".into()), found_crate);

    input
}
