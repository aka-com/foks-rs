use crate::{Config, Database, MaintenanceReport};
use rusqlite::params;

pub const NOW: u64 = 86_400_010_000;
pub const USER: [u8; 33] = [1; 33];
pub const TEAM: [u8; 33] = [2; 33];
pub const HOST: [u8; 33] = [3; 33];
const KEY: [u8; 16] = [4; 16];

pub fn sql(value: u64) -> i64 {
    crate::error::sql_integer(value).unwrap()
}

pub fn id<const N: usize>(value: u64) -> [u8; N] {
    let mut bytes = [0; N];
    bytes[..8].copy_from_slice(&value.to_be_bytes());
    bytes
}

pub fn cutoff(table: &str) -> u64 {
    match table {
        "sso_sessions" => NOW / 1000,
        "log_sends" => NOW.saturating_sub(86_400_000_000),
        _ => NOW,
    }
}

pub fn expiry_column(table: &str) -> &'static str {
    match table {
        "sso_sessions" => "expires_at_ms",
        "log_sends" => "created_at",
        _ => "expires_at",
    }
}

pub struct Fixture {
    pub db: Database,
    _dir: tempfile::TempDir,
}
impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("expiry.sqlite"), Config::default()).unwrap();
        db.connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        assert!(db
            .connection
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap());
        db.connection
            .execute(
                "INSERT INTO names VALUES(x'706172656e74',NULL,1,NULL,0,?1)",
                [USER],
            )
            .unwrap();
        db.connection.execute("INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES(?1,x'706172656e74',x'506172656e74',1,zeroblob(16),0)", [USER]).unwrap();
        db.connection
            .execute(
                "INSERT INTO teams VALUES(?1,20,?2,NULL,x'7465616d',0,NULL,1,0,0)",
                params![TEAM, HOST],
            )
            .unwrap();
        db.connection
            .execute(
                "INSERT INTO capability_key_generations VALUES(?1,'fixture.key',1,0,NULL)",
                [KEY],
            )
            .unwrap();
        let f = Self { db, _dir: dir };
        f.policy();
        f
    }
    pub fn policy(&self) {
        self.db.connection.execute("INSERT INTO sso_policy VALUES(?1,zeroblob(32),zeroblob(16),'https://issuer.test',0,NULL,1,1)",[HOST]).unwrap();
    }
    pub fn seed(&mut self, table: &str, start: u64, count: usize, expires: u64) {
        let expires = sql(expires);
        let tx = self.db.connection.transaction().unwrap();
        let insert = match table {
            "names" => "INSERT INTO names(normalized_name,reservation_token,reservation_sequence,expires_at) VALUES(?1,?2,1,?3)",
            "team_names" => "INSERT INTO team_names(normalized_name,reservation_token,reservation_sequence,expires_at) VALUES(?1,?2,1,?3)",
            "recovery_challenges" => "INSERT INTO recovery_challenges VALUES(?1,?2,?3,?4,?5,0)",
            "team_view_challenges" => "INSERT INTO team_view_challenges VALUES(?1,?1,?2,?3,?4,1,0,1,?5,?6,0,NULL)",
            "team_view_tokens" => "INSERT INTO team_view_tokens VALUES(?1,?2,?3,?4,1,0,1,1,0,?5)",
            "team_admin_tokens" => "INSERT INTO team_admin_tokens VALUES(?1,?2,?3,2,0,1,?4,NULL)",
            "sso_sessions" => "INSERT INTO sso_sessions VALUES(?1,?2,zeroblob(32),?2,NULL,0,1,1,0,?3,zeroblob(57))",
            "request_receipts" => "INSERT INTO request_receipts VALUES(?1,zeroblob(32),x'00',0,?2)",
            "log_sends" => "INSERT INTO log_sends VALUES(?1,NULL,?2)",
            _ => panic!("unknown expiry kind {table}"),
        };
        // One prepared statement per transaction, rather than preparing each row.
        let mut statement = tx.prepare(insert).unwrap();
        for i in start..start + count as u64 {
            match table {
                "names" | "team_names" => statement.execute(params![
                    format!("reservation-{i}").as_bytes(),
                    id::<17>(i),
                    expires
                ]),
                "recovery_challenges" => {
                    statement.execute(params![id::<32>(i), USER, HOST, KEY, expires])
                }
                "team_view_challenges" => {
                    statement.execute(params![id::<32>(i), TEAM, USER, HOST, KEY, expires])
                }
                "team_view_tokens" => {
                    statement.execute(params![id::<32>(i), TEAM, USER, HOST, expires])
                }
                "team_admin_tokens" => statement.execute(params![id::<32>(i), TEAM, USER, expires]),
                "sso_sessions" => statement.execute(params![HOST, id::<32>(i), expires]),
                "request_receipts" => statement.execute(params![id::<16>(i), expires]),
                "log_sends" => statement.execute(params![id::<17>(i), expires]),
                _ => unreachable!(),
            }
            .unwrap();
        }
        drop(statement);
        tx.commit().unwrap();
    }
    pub fn set_expiry(&self, table: &str, index: u64, expiry: u64) {
        let (key, bytes) = match table {
            "names" | "team_names" => (
                "normalized_name",
                format!("reservation-{index}").into_bytes(),
            ),
            "recovery_challenges" | "team_view_challenges" => {
                ("challenge_hash", id::<32>(index).to_vec())
            }
            "sso_sessions" => ("session_hash", id::<32>(index).to_vec()),
            "request_receipts" => ("idempotency_key", id::<16>(index).to_vec()),
            "log_sends" => ("log_send_id", id::<17>(index).to_vec()),
            _ => ("token_hash", id::<32>(index).to_vec()),
        };
        self.db
            .connection
            .execute(
                &format!(
                    "UPDATE {table} SET {}=?1 WHERE {key}=?2",
                    expiry_column(table)
                ),
                params![sql(expiry), bytes],
            )
            .unwrap();
    }
    pub fn count(&self, table: &str) -> usize {
        let extra = if table == "names" {
            " WHERE normalized_name<>x'706172656e74'"
        } else {
            ""
        };
        let count: i64 = self
            .db
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}{extra}"), [], |r| {
                r.get(0)
            })
            .unwrap();
        usize::try_from(count).unwrap()
    }
    pub fn integrity(&self) {
        assert!(!self
            .db
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
    }
}

pub fn counts(report: MaintenanceReport) -> [(&'static str, u64); 9] {
    [
        ("sso_sessions", report.sso_sessions),
        ("names", report.reservations),
        ("team_names", report.team_reservations),
        ("request_receipts", report.receipts),
        ("recovery_challenges", report.challenges),
        ("team_view_tokens", report.team_view_tokens),
        ("team_view_challenges", report.team_view_challenges),
        ("team_admin_tokens", report.team_admin_tokens),
        ("log_sends", report.log_sends),
    ]
}
