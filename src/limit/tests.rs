use std::net::{IpAddr, Ipv4Addr};

use super::{Limit, Limiter};

const ONE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));
const TWO: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 21));

#[test]
fn the_standard_limit_is_five_a_minute() {
    assert_eq!(
        Limit::standard(),
        Limit {
            asks: 5,
            window: 60
        }
    );
}

#[test]
fn an_address_is_let_through_up_to_the_limit_and_no_further() {
    let mut limiter = Limiter::new(Limit::standard());

    let admitted: Vec<bool> = (0..6).map(|at| limiter.admits(ONE, 100 + at)).collect();

    assert_eq!(admitted, vec![true, true, true, true, true, false]);
}

#[test]
fn each_address_has_its_own_count() {
    let mut limiter = Limiter::new(Limit {
        asks: 1,
        window: 60,
    });

    assert!(limiter.admits(ONE, 100));
    assert!(limiter.admits(TWO, 100));
    assert!(!limiter.admits(ONE, 101));
}

#[test]
fn a_window_that_has_passed_is_forgotten() {
    let mut limiter = Limiter::new(Limit {
        asks: 1,
        window: 60,
    });

    assert!(limiter.admits(ONE, 100));
    assert!(!limiter.admits(ONE, 159));
    assert!(limiter.admits(ONE, 160));
}
