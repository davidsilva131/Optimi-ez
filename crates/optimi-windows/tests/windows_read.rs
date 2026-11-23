#![cfg(windows)]

use optimi_domain::adapter::SystemAdapter;
use optimi_windows::WindowsSystemAdapter;

#[test]
#[ignore = "requires disposable Windows 11 VM"]
fn reads_real_windows_state_without_writing() {
    let adapter = WindowsSystemAdapter;
    let version = adapter
        .windows_version()
        .expect("windows version must be readable");
    assert!(
        version.supports_current_release(),
        "this test requires a supported Windows 11 client, found {version:?}"
    );

    let identity = adapter
        .interactive_identity()
        .expect("interactive identity must be readable");
    assert!(!identity.account.is_empty());

    let plan = adapter
        .active_power_plan()
        .expect("active power plan must be readable");
    assert!(!plan.guid.is_empty());

    let service = adapter
        .service_state("DiagTrack")
        .expect("DiagTrack service state must be readable");

    println!(
        "windows={}.{}.{}",
        version.major, version.minor, version.build
    );
    println!("identity={:?}\\{}", identity.domain, identity.account);
    println!("power_plan={:?} ({})", plan.name, plan.guid);
    println!(
        "diagtrack run_state={:?} start_type={:?}",
        service.run_state, service.start_type
    );
}
