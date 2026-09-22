//! The remote database drivers a generated project can carry, pinned once.
//!
//! Two deploy-layer slots dial a database, and they dial the same two servers: a
//! target's `journal:` (grammar §14.7, PRD resolved q62) and a `kv` store's
//! `storage_backends:` entry (grammar §14.3, PRD resolved q63). Both reach
//! Postgres through `pg` and MySQL through `mysql2`, so the pin is **one** table
//! here rather than one beside each slot.
//!
//! That is not tidiness. A project may bind both at once — the journal on
//! Postgres and a store on the same server is the ordinary deployment — and a
//! `package.json` is a map: two tables would agree on the day they were written
//! and could disagree later, and the disagreement would be a manifest declaring
//! one package at two versions, which is the one thing [`super::project`] panics
//! over rather than emitting. One table makes "one dependency entry when both
//! bind" true by construction.
//!
//! The pins themselves are under PRD §9.18's discipline, like every other pin
//! this compiler writes: what a driver does is what a compiled graph survives, so
//! a release that let one float would change that with no commit saying so
//! (PRD §5.12).

/// One driver a generated project can carry.
///
/// Named after the package rather than after the server, because the package is
/// what a manifest holds and what a slot's arm imports: `journal-postgres.ts`
/// and `stores-postgres.ts` are two modules over one `pg`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemoteDriver {
    /// `pg` — what both Postgres arms dial with.
    Pg,
    /// `mysql2` — what both MySQL arms dial with.
    Mysql2,
}

impl RemoteDriver {
    /// Every driver, in the order a provider vocabulary lists its servers.
    pub const ALL: &'static [Self] = &[Self::Pg, Self::Mysql2];

    /// The package this driver is, as a manifest spells it.
    #[must_use]
    pub const fn package(self) -> &'static str {
        match self {
            Self::Pg => "pg",
            Self::Mysql2 => "mysql2",
        }
    }
}

/// The runtime packages one driver brings, pinned exactly.
pub const DRIVER_PINS: &[(RemoteDriver, &[(&str, &str)])] = &[
    (RemoteDriver::Pg, &[("pg", "8.23.0")]),
    (RemoteDriver::Mysql2, &[("mysql2", "3.24.4")]),
];

/// …and the development ones.
///
/// One entry has ever arrived here and it is `@types/pg`: `pg` publishes no type
/// declarations of its own, and an emitted module importing it under `strict`
/// fails `tsc` on the import rather than on anything this compiler wrote.
/// `mysql2` ships its own, so it has no row.
pub const DRIVER_DEV_PINS: &[(RemoteDriver, &[(&str, &str)])] =
    &[(RemoteDriver::Pg, &[("@types/pg", "8.23.1")])];

/// The packages one driver brings.
#[must_use]
pub fn pins_of(driver: RemoteDriver) -> &'static [(&'static str, &'static str)] {
    DRIVER_PINS
        .iter()
        .find(|(held, _)| *held == driver)
        .map_or(&[], |(_, pins)| *pins)
}

/// …and the development ones.
#[must_use]
pub fn development_pins_of(driver: RemoteDriver) -> &'static [(&'static str, &'static str)] {
    DRIVER_DEV_PINS
        .iter()
        .find(|(held, _)| *held == driver)
        .map_or(&[], |(_, pins)| *pins)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every driver is pinned, and each package is pinned once.
    ///
    /// The second half is the one that matters: this table exists so that a
    /// project binding a journal and a store on one server declares one entry,
    /// and a package listed twice here at two versions would make that
    /// [`super::project::dependencies`]'s panic instead.
    #[test]
    fn every_driver_pins_its_package_exactly_once() {
        let mut seen: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
        for driver in RemoteDriver::ALL {
            assert!(
                pins_of(*driver)
                    .iter()
                    .any(|(package, _)| *package == driver.package()),
                "`{}` is a driver this compiler emits an arm for and pins nothing",
                driver.package()
            );
            for (package, version) in pins_of(*driver).iter().chain(development_pins_of(*driver)) {
                let held = seen.insert(package, version);
                assert!(
                    held.is_none(),
                    "`{package}` is pinned by two drivers, so a project binding both would \
                     declare it twice"
                );
            }
        }
    }
}
