/// Tests for the command line interface (e.g. `nvim-gtk --no-fork foo.txt`)

#[test]
fn cli_tests() {
    trycmd::TestCases::new()
        .env("NVIM_GTK_CLI_TEST_MODE", "1")
        .case("tests/cmd/*.trycmd");
}
