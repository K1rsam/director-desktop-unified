//! Normal-browser opening is separate from app-owned Edge/CDP and login.
use super::*;

#[test]
fn normal_browser_requested_family_is_explicit_and_legacy_default_stays_chrome() {
    assert_eq!(NormalBrowser::requested(None).unwrap(), NormalBrowser::Chrome);
    assert_eq!(NormalBrowser::requested(Some("chrome")).unwrap(), NormalBrowser::Chrome);
    assert_eq!(NormalBrowser::requested(Some("edge")).unwrap(), NormalBrowser::Edge);
    for value in ["", "Edge", "firefox", "edge --enable-automation", "https://other.test", "../edge"] {
        assert!(NormalBrowser::requested(Some(value)).is_err());
    }
    assert_eq!(NormalBrowser::Edge.application(), "Microsoft/Edge/Application/msedge.exe");
    assert_eq!(NormalBrowser::Edge.user_data(), "Microsoft/Edge/User Data");
    assert_eq!(NormalBrowser::Chrome.user_data(), "Google/Chrome/User Data");
}

#[test]
fn normal_browser_launch_has_no_automation_debugging_or_owned_profile_arguments() {
    assert_eq!(normal_browser_args(None).unwrap(), vec!["https://chatgpt.com/"]);
    for profile in ["Default", "Profile 1", "Profile 16"] {
        let args = normal_browser_args(Some(profile)).unwrap();
        assert_eq!(args, vec![format!("--profile-directory={profile}"), "https://chatgpt.com/".into()]);
        for prohibited in ["automation", "remote-debugging", "headless", "user-data-dir", "load-extension", "disable-extensions", "disable-blink", "new-window"] {
            assert!(args.iter().all(|argument| !argument.contains(prohibited)));
        }
    }
    for profile in ["Profile 0", "Profile 17", "Profile 1 --headless", "../Default", "", "default"] {
        assert!(normal_browser_args(Some(profile)).is_err());
    }
}

#[test]
fn normal_browser_profile_lookup_preserves_files_and_uses_the_exact_extension() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("ordinary-edge");
    let extension = temp.path().join("verified-extension");
    let other = temp.path().join("other-extension");
    fs::create_dir_all(&extension).unwrap();
    fs::create_dir_all(&other).unwrap();
    let default = root.join("Default");
    let selected = root.join("Profile 3");
    fs::create_dir_all(&default).unwrap();
    fs::create_dir_all(&selected).unwrap();
    let preferences = |path: &Path| json!({"extensions":{"settings":{"test":{"path":path}}}}).to_string();
    let default_bytes = preferences(&other);
    let selected_bytes = preferences(&extension);
    fs::write(default.join("Preferences"), &default_bytes).unwrap();
    fs::write(selected.join("Secure Preferences"), &selected_bytes).unwrap();
    fs::write(selected.join("Cookies"), b"synthetic-private-login-file-do-not-read-or-copy").unwrap();
    assert_eq!(profile_with_extension(&root, &extension), Some("Profile 3".into()));
    assert_eq!(fs::read_to_string(default.join("Preferences")).unwrap(), default_bytes);
    assert_eq!(fs::read_to_string(selected.join("Secure Preferences")).unwrap(), selected_bytes);
    assert_eq!(fs::read(selected.join("Cookies")).unwrap(), b"synthetic-private-login-file-do-not-read-or-copy");
    assert_eq!(fs::read_dir(selected).unwrap().count(), 2);
    assert!(!root.join("managed-edge").exists());
}

#[test]
fn normal_browser_missing_or_unrelated_extension_keeps_the_browsers_own_default() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("ordinary-profile");
    let extension = temp.path().join("verified-extension");
    assert_eq!(profile_with_extension(&root, &extension), None);
    assert!(!root.exists());
    fs::create_dir_all(root.join("Default")).unwrap();
    let original = "{not-valid-preferences";
    fs::write(root.join("Default/Preferences"), original).unwrap();
    assert_eq!(profile_with_extension(&root, &extension), None);
    assert_eq!(fs::read_to_string(root.join("Default/Preferences")).unwrap(), original);
    assert!(!extension.exists());
}
