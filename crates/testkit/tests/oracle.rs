use worklens_testkit::screen_problems;

const OFF: &[&str] = &["jira", "phabricator"];

#[test]
fn screens_that_shipped_with_a_lie_are_still_caught() {
    let lies = [
        "│ Unknown      telemetry, phabricator (fetch failed)       │",
        "│ Fetching your work from Bugzilla, Jira and Phabricator…  │",
        "│ Linking Jira tickets to bugs                             │",
        "No sprint: Jira wasn't read.",
        "Drift needs Jira, which couldn't be read; see `worklens doctor`.",
        "│ Off: Jira isn't set up.        │\n│ Linking Jira tickets to bugs   │",
        "│ ~ jira      no token          │\n│ Linking Jira tickets to bugs  │",
        "phabricator: not set up\nLinking Jira tickets to bugs",
        "│ list │ Off: Jira isn't set up. │\n│ D123 │ Phabricator revision    │",
    ];
    for screen in lies {
        assert!(
            !screen_problems(screen, OFF, false).is_empty(),
            "this screen lies and must be caught:\n{screen}"
        );
    }
}

#[test]
fn honest_screens_pass() {
    let honest = [
        "│ Off: Phabricator isn't set up. `worklens auth set        │\n│ phabricator` turns it on. s opens Sources.               │",
        "jira: not set up, so Sprint, Ideas and Drift are off; `worklens auth set jira` turns them on",
        "│ Off: Phabricator isn't set up.          │\n│ `worklens auth set phabricator` turns it on. │",
        "│ 1995008 Support view-source │ Off: Jira isn't set up. `worklens │\n│ 2044168 Improve logging     │ auth set jira` turns it on.        │",
        "Waiting on you 1\n   7d  ni?  2086888     Remove metrics",
    ];
    for screen in honest {
        assert_eq!(
            screen_problems(screen, OFF, false),
            Vec::<String>::new(),
            "this screen is honest:\n{screen}"
        );
    }
    let failure = "│ Not checked: Jira couldn't be read. │";
    assert_eq!(
        screen_problems(failure, &[], true),
        Vec::<String>::new(),
        "a real failure may say so"
    );
    assert!(!screen_problems(failure, &[], false).is_empty(), "but not when nothing failed");
}
