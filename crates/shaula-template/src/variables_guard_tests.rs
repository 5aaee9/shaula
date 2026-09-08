use super::discover_variables;
use super::tests::fixture;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn deeply_nested_source_is_rejected_before_the_recursive_parser() -> TestResult {
    for expression in [
        format!("{}0{}", "(".repeat(50_000), ")".repeat(50_000)),
        format!("{}true", "!".repeat(50_000)),
        format!("{}0", "true ? 0 : ".repeat(10_000)),
        format!("{}0", "0 + ".repeat(10_000)),
        format!(
            "\"{}ok{}\"",
            "%{ if true }".repeat(10_000),
            "%{ endif }".repeat(10_000)
        ),
    ] {
        let directory = fixture("", r#"{"type":"object","properties":{}}"#)?;
        std::fs::write(
            directory.path().join("main.tf"),
            format!("locals {{\n value = {expression}\n}}\n"),
        )?;
        let error = discover_variables(directory.path(), "digest")
            .err()
            .ok_or("deep source accepted")?;
        assert_eq!(
            error.summary,
            "Terraform source exceeds the syntax complexity limit"
        );
    }
    Ok(())
}

#[test]
fn strings_comments_and_heredocs_do_not_count_literal_brackets() -> TestResult {
    let directory = fixture("", r#"{"type":"object","properties":{}}"#)?;
    let brackets = "[".repeat(1_000);
    let main = format!(
        r#"
// {brackets}
# {brackets}
/* {brackets} */
locals {{
  quoted = "{brackets} \" $${{not_interpolated}}"
  heredoc = <<-EOT
    {brackets} $${{not_interpolated}}
    EOT
  interpolation = "foo${{upper("bar")}}"
}}
"#
    );
    std::fs::write(directory.path().join("main.tf"), main)?;
    assert!(discover_variables(directory.path(), "digest")?.available);
    Ok(())
}

#[test]
fn heredoc_interpolations_remain_subject_to_the_depth_budget() -> TestResult {
    let directory = fixture("", r#"{"type":"object","properties":{}}"#)?;
    let expression = format!("{}0{}", "(".repeat(1_000), ")".repeat(1_000));
    let main = format!("locals {{\n value = <<EOT\n${{{expression}}}\nEOT\n}}\n");
    std::fs::write(directory.path().join("main.tf"), main)?;
    assert!(discover_variables(directory.path(), "digest").is_err());
    Ok(())
}
