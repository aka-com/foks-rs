use crate::catalog::{Kind, Spec, Statement};
use crate::MAX_BLOB_BYTES;

pub fn spec(statement: Statement) -> Spec {
    use Kind::{Blob as B, Integer as I, NullableBlob as N, Text as T};
    use Statement::*;
    let (sql, parameters, columns, write): (_, &'static [_], &'static [_], _) = match statement {
        AuthorityState => ("SELECT identity,activation,owner,CAST(epoch AS TEXT),CAST(revision AS TEXT),CAST(high_water AS TEXT),CAST(retained_rows AS TEXT),CAST(reserved_bytes AS TEXT),CAST(max_rows AS TEXT),CAST(max_bytes AS TEXT),CAST(clock_floor AS TEXT) FROM metadata_authority WHERE singleton=1", &[], &[T,T,B,I,I,I,I,I,I,I,I], false),
        AuthorityInitialize => ("INSERT INTO metadata_authority(singleton,identity,activation,owner,epoch,revision,high_water,max_rows,max_bytes,clock_floor) VALUES(1,?1,?2,?3,1,1,0,CAST(?4 AS INTEGER),CAST(?5 AS INTEGER),CAST(?6 AS INTEGER))", &[T,T,B,I,I,I], &[], true),
        AuthorityClock => ("UPDATE metadata_authority SET clock_floor=CAST(?1 AS INTEGER) WHERE singleton=1", &[I], &[], true),
        AuthorityPromote => ("UPDATE metadata_authority SET owner=?1,epoch=epoch+1,revision=revision+1,high_water=0 WHERE singleton=1 AND epoch=CAST(?2 AS INTEGER) AND owner=?3", &[B,I,B], &[], true),
        AuthorityActivate => ("UPDATE metadata_authority SET activation=?1,revision=revision+1 WHERE singleton=1", &[T], &[], true),
        AuthorityRevision => ("UPDATE metadata_authority SET revision=revision+1 WHERE singleton=1", &[], &[], true),
        AuthorityAllocate => ("UPDATE metadata_authority SET high_water=CAST(?1 AS INTEGER) WHERE singleton=1 AND high_water=CAST(?2 AS INTEGER)", &[I,I], &[], true),
        AuthorityPrincipal => ("SELECT namespace,role,CAST(enabled AS TEXT) FROM metadata_principals WHERE principal=?1", &[B], &[B,B,I], false),
        AuthoritySetPrincipal => ("INSERT INTO metadata_principals(principal,namespace,role,enabled) VALUES(?1,?2,?3,CAST(?4 AS INTEGER)) ON CONFLICT(principal) DO UPDATE SET namespace=excluded.namespace,role=excluded.role,enabled=excluded.enabled", &[B,B,B,I], &[], true),
        AuthorityIntent => ("SELECT digest,principal,namespace,CAST(revision AS TEXT),CAST(expires AS TEXT),CAST(terminal AS TEXT),outcome FROM metadata_intents WHERE epoch=CAST(?1 AS INTEGER) AND sequence=CAST(?2 AS INTEGER)", &[I,I], &[B,B,B,I,I,I,N], false),
        AuthorityRegister => ("INSERT INTO metadata_intents(epoch,sequence,digest,principal,namespace,revision,expires) VALUES(CAST(?1 AS INTEGER),CAST(?2 AS INTEGER),?3,?4,?5,CAST(?6 AS INTEGER),CAST(?7 AS INTEGER))", &[I,I,B,B,B,I,I], &[], true),
        AuthorityOutcome => ("UPDATE metadata_intents SET terminal=1,outcome=?1 WHERE epoch=CAST(?2 AS INTEGER) AND sequence=CAST(?3 AS INTEGER) AND digest=?4 AND terminal=0", &[B,I,I,B], &[], true),
        AuthoritySettle => ("UPDATE metadata_intents SET terminal=1,outcome=?1 WHERE (epoch,sequence) IN (SELECT epoch,sequence FROM metadata_intents WHERE terminal=0 AND (epoch<CAST(?2 AS INTEGER) OR expires<=CAST(?3 AS INTEGER) OR revision!=CAST(?4 AS INTEGER)) ORDER BY epoch,sequence LIMIT CAST(?5 AS INTEGER))", &[B,I,I,I,I], &[], true),
        AuthorityPrune => ("DELETE FROM metadata_intents WHERE (epoch,sequence) IN (SELECT epoch,sequence FROM metadata_intents WHERE terminal=1 ORDER BY epoch,sequence LIMIT CAST(?1 AS INTEGER))", &[I], &[], true),
        AuthoritySavepoint => ("SAVEPOINT metadata_domain", &[], &[], true),
        AuthorityRollback => ("ROLLBACK TO metadata_domain", &[], &[], true),
        AuthorityRelease => ("RELEASE metadata_domain", &[], &[], true),
        _ => unreachable!("only authority catalog variants delegate here"),
    };
    Spec {
        id: statement as u32,
        sql,
        parameters,
        columns,
        write,
        maximum_blob_bytes: MAX_BLOB_BYTES,
    }
}
