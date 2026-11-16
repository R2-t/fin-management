use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};

/// Argon2id parameters, the OWASP baseline: 19 MiB of memory, two passes, one
/// lane. The PHC string stores them, so raising the cost later still verifies
/// hashes written today.
const MEMORY_COST_KIB: u32 = 19_456;
const TIME_COST: u32 = 2;
const PARALLELISM: u32 = 1;

/// A hash written for an email that does not exist, so a login for an unknown
/// address costs the same time as a wrong password. Without it, "no such user"
/// returns in microseconds and "wrong password" after a 19 MiB hash -- a
/// working oracle for which emails have accounts.
///
/// It is computed once from random bytes rather than hard-coded: a literal
/// that fails to parse would skip the hash entirely and restore the very
/// timing difference it exists to remove.
fn dummy_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        let unusable = uuid::Uuid::new_v4().to_string();
        hash_password(&unusable).expect("the dummy hash is computable")
    })
}

fn params() -> Params {
    Params::new(MEMORY_COST_KIB, TIME_COST, PARALLELISM, None)
        .expect("hard-coded Argon2id parameters are valid")
}

pub fn argon2() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params())
}

/// Hashes a password into a PHC string.
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    argon2()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("password could not be hashed: {e}"))
}

/// Verifies a password against a stored PHC string.
///
/// A hash that does not parse (a seeded or disabled user) is a failed
/// verification, not an error: the caller must return the same answer as a
/// wrong password, so the two cases are indistinguishable from outside.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_hash) else {
        return false;
    };
    argon2()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Burns the same work a real verification would, for an email with no row.
pub fn verify_against_dummy(password: &str) {
    let _ = verify_password(password, dummy_hash());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_hashes_and_verifies() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &hash));
    }

    #[test]
    fn a_wrong_password_fails() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(!verify_password("correct horse battery stapl", &hash));
    }

    #[test]
    fn an_unparseable_hash_fails_closed_instead_of_panicking() {
        assert!(!verify_password("anything", "!disabled"));
        assert!(!verify_password("anything", ""));
    }

    #[test]
    fn the_dummy_hash_parses_so_it_actually_burns_the_work() {
        assert!(PasswordHash::new(dummy_hash()).is_ok());
        assert!(!verify_password("anything", dummy_hash()));
    }

    #[test]
    fn the_hash_carries_the_parameters_the_doc_names() {
        let hash = hash_password("a long enough password").unwrap();
        assert!(hash.contains("m=19456,t=2,p=1"), "{hash}");
    }
}
