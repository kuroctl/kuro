//! Ce qui relie les instances d'une même installation (ADR-0037) : le bus de notifications
//! Postgres, par lequel une instance en réveille une autre. L'identité des instances est dans
//! `nodes`, le relais des commandes dans `ws`.

pub mod bus;

pub use bus::Bus;
