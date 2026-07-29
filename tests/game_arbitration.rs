//! Интеграционные тесты арбитража владения игровым слотом (GameSlot). Не
//! требует feature "app" — game_state безусловный модуль библиотеки,
//! доступен как aurora_vibra::game_state::GameSlot из любой сборки.

use aurora_vibra::types::ActiveGame;
use parking_lot::Mutex;
use std::sync::Arc;

use aurora_vibra::game_state::GameSlot;

#[test]
fn sticky_ownership_no_double_claim() {
    let active = Arc::new(Mutex::new(ActiveGame::None));
    let msfs = GameSlot::new(active.clone());
    let wt = GameSlot::new(active.clone());

    assert!(msfs.try_claim(ActiveGame::Msfs));
    assert!(!wt.try_claim(ActiveGame::Wt));
    assert!(msfs.try_claim(ActiveGame::Msfs)); // повторный claim собой — идемпотентно
    assert_eq!(*active.lock(), ActiveGame::Msfs);

    wt.release_if_owned(ActiveGame::Wt); // не владеет — no-op
    assert_eq!(*active.lock(), ActiveGame::Msfs);

    msfs.release_if_owned(ActiveGame::Msfs);
    assert_eq!(*active.lock(), ActiveGame::None);

    assert!(wt.try_claim(ActiveGame::Wt));
    assert!(!msfs.try_claim(ActiveGame::Msfs));
}

#[test]
fn interleaved_claims_never_produce_double_ownership() {
    let active = Arc::new(Mutex::new(ActiveGame::None));
    let a = GameSlot::new(active.clone());
    let b = GameSlot::new(active.clone());
    let ops: &[(bool, ActiveGame)] = &[
        (true, ActiveGame::Msfs),
        (true, ActiveGame::Wt),
        (true, ActiveGame::Wt),
        (false, ActiveGame::Msfs),
        (true, ActiveGame::Wt),
        (false, ActiveGame::Wt),
        (true, ActiveGame::Msfs),
    ];
    for &(claim, who) in ops {
        if claim {
            let _ = if who == ActiveGame::Msfs {
                a.try_claim(who)
            } else {
                b.try_claim(who)
            };
        } else if who == ActiveGame::Msfs {
            a.release_if_owned(who)
        } else {
            b.release_if_owned(who)
        };
        assert!(matches!(
            *active.lock(),
            ActiveGame::None | ActiveGame::Msfs | ActiveGame::Wt
        ));
    }
}
