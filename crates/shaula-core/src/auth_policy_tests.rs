//! Spec 0011 §2–3 matching, validation and canonical-order tests.

use super::*;
use crate::github::GitHubTarget;

fn org(name: &str) -> GitHubTarget {
    GitHubTarget::organization(name).unwrap()
}

fn repo(owner: &str, name: &str) -> GitHubTarget {
    GitHubTarget::new_repository(owner, name).unwrap()
}

#[test]
fn organization_selector_admits_only_the_org_target() {
    let selector = TargetSelector::organization("Indexyz").unwrap();
    assert!(selector.allows(&org("indexyz")), "case-insensitive match");
    assert!(!selector.allows(&repo("Indexyz", "shaula")));
    assert!(!selector.allows(&org("other")));
}

#[test]
fn repository_selector_admits_only_that_exact_repository() {
    let selector = TargetSelector::repository("5aaee9", "new-project").unwrap();
    assert!(selector.allows(&repo("5AAEE9", "New-Project")));
    assert!(!selector.allows(&org("5aaee9")));
    assert!(!selector.allows(&repo("5aaee9", "other")));
    assert!(!selector.allows(&repo("other", "new-project")));
}

#[test]
fn account_repositories_admits_owned_repositories_not_the_org_target() {
    let user = TargetSelector::account_repositories(AccountKind::User, "5aaee9").unwrap();
    assert!(user.allows(&repo("5aaee9", "anything")));
    assert!(!user.allows(&org("5aaee9")), "org target never implied");
    // A repository the account only collaborates on has another owner.
    assert!(!user.allows(&repo("someone-else", "shared")));
}

#[test]
fn selectors_reject_urls_globs_and_path_fragments() {
    assert!(TargetSelector::organization("https://github.com/x").is_err());
    assert!(TargetSelector::organization("*").is_err());
    assert!(TargetSelector::organization("../etc").is_err());
    assert!(TargetSelector::organization("a/b").is_err());
    assert!(TargetSelector::repository("o", "a/b").is_err());
    assert!(TargetSelector::repository("o", "*").is_err());
    assert!(TargetSelector::account_repositories(AccountKind::Organization, "").is_err());
    // Reserved owner name keeps the protocol path unambiguous.
    assert!(TargetSelector::organization("enterprises").is_err());
}

#[test]
fn serde_rejects_unknown_fields_and_mismatched_shapes() {
    let parsed: TargetSelector =
        serde_json::from_str(r#"{"kind":"organization","owner":"o"}"#).unwrap();
    assert_eq!(parsed, TargetSelector::organization("o").unwrap());
    assert!(serde_json::from_str::<TargetSelector>(
        r#"{"kind":"organization","owner":"o","repository":"x"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<TargetSelector>(r#"{"kind":"glob","pattern":"*"}"#).is_err());
    assert!(serde_json::from_str::<TargetSelector>(
        r#"{"kind":"account_repositories","account_kind":"user","owner":"o","extra":1}"#
    )
    .is_err());
}

#[test]
fn policy_is_canonically_ordered_and_deduplicated() {
    let a = TargetSelector::organization("Beta").unwrap();
    let b = TargetSelector::organization("alpha").unwrap();
    let c = TargetSelector::account_repositories(AccountKind::User, "5aaee9").unwrap();
    let policy = TargetPolicy::new(vec![a.clone(), c.clone(), b.clone()]).unwrap();
    let owners: Vec<String> = policy
        .selectors()
        .iter()
        .map(|s| s.account().to_ascii_lowercase())
        .collect();
    assert_eq!(owners, vec!["alpha", "beta", "5aaee9"]);

    // A case variant of an existing selector is a duplicate.
    let case_variant = TargetSelector::organization("ALPHA").unwrap();
    assert!(TargetPolicy::new(vec![b.clone(), case_variant]).is_err());
    // Byte-equal duplicates are rejected too.
    assert!(TargetPolicy::new(vec![b.clone(), b.clone()]).is_err());

    // Order permutations serialize identically, so replay hashes stay
    // stable across equal-set publications.
    let other = TargetPolicy::new(vec![c.clone(), b.clone(), a.clone()]).unwrap();
    assert_eq!(
        serde_json::to_string(&policy).unwrap(),
        serde_json::to_string(&other).unwrap()
    );
}

#[test]
fn policy_bounds_are_enforced() {
    assert!(TargetPolicy::new(vec![]).is_err());
    let many: Vec<TargetSelector> = (0..=MAX_SELECTORS)
        .map(|i| TargetSelector::organization(format!("org{i}")).unwrap())
        .collect();
    assert!(TargetPolicy::new(many).is_err(), "101 selectors rejected");
    let accounts: Vec<TargetSelector> = (0..MAX_ACCOUNTS + 1)
        .map(|i| TargetSelector::account_repositories(AccountKind::User, format!("u{i}")).unwrap())
        .collect();
    assert!(
        TargetPolicy::new(accounts).is_err(),
        "51 distinct accounts rejected"
    );
}

#[test]
fn policy_matching_lists_every_selector_for_ambiguity_detection() {
    let policy = TargetPolicy::new(vec![
        TargetSelector::repository("o", "r").unwrap(),
        TargetSelector::account_repositories(AccountKind::User, "O").unwrap(),
        TargetSelector::organization("other").unwrap(),
    ])
    .unwrap();
    assert_eq!(policy.matches(&repo("o", "r")).len(), 2);
    assert_eq!(policy.matches(&org("other")).len(), 1);
    assert!(policy.matches(&repo("other", "x")).is_empty());
    assert!(!policy.allows(&org("o")));
}
