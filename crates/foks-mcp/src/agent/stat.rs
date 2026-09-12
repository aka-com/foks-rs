//! Go's JSON presentation of KVStat, independent of remote Snowpack encoding.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use foks_agent_proto::data::DataStat;
use foks_proto::{
    encode_base62_strict, KvDirectoryPair, KvDirent, KvNode, KvNodeType, Role, RoleType,
};
use serde_json::{json, Value};

fn role(role: Role) -> Result<String, String> {
    Ok(match role.kind() {
        RoleType::Owner => "OWNER".into(),
        RoleType::Admin => "ADMIN".into(),
        RoleType::Member => format!("MEMBER/{}", role.visibility().ok_or("missing visibility")?),
        RoleType::None => "NONE".into(),
    })
}

pub(super) fn go_stat(evidence: DataStat) -> Result<Value, String> {
    let invalid = |_| "invalid stat evidence".to_owned();
    let dirent = evidence
        .dirent
        .as_deref()
        .map(KvDirent::decode)
        .transpose()
        .map_err(invalid)?;
    let (variable, key, write) = if let Some(directory) = &evidence.directory {
        let directory = KvDirectoryPair::decode(directory).map_err(invalid)?.active;
        (
            json!({"T":1,"f1":{"Vers":directory.version}}),
            directory.key,
            directory.write_role,
        )
    } else {
        let entry = dirent.as_ref().ok_or("missing stat dirent")?;
        let node = KvNode::decode(evidence.node.as_deref().ok_or("missing stat metadata")?)
            .map_err(invalid)?;
        match node {
            KvNode::File(metadata) => (
                json!({"T":2,"f2":{"Size":0}}),
                metadata.key,
                entry.write_role,
            ),
            KvNode::SmallFile(boxed) => (
                json!({"T":3,"f2":{"Size":evidence.size.ok_or("missing file size")?}}),
                boxed.key,
                entry.write_role,
            ),
            KvNode::Symlink(boxed) => (
                json!({"T":4,"f4":{"Target":evidence.target.ok_or("missing symlink target")?}}),
                boxed.key,
                entry.write_role,
            ),
            _ => return Err("unexpected stat metadata".into()),
        }
    };
    let de = match dirent {
        None => Value::Null,
        Some(entry) => {
            let kind = entry.value.node_type().map_err(invalid)?;
            let tag = match kind {
                KvNodeType::Directory => '1',
                KvNodeType::SmallFile => '3',
                KvNodeType::File => '2',
                KvNodeType::Symlink => '4',
                _ => return Err("invalid stat node".into()),
            };
            json!({"ParentDir":entry.parent,"Id":encode_base62_strict(&entry.id),
                "Value":format!("{tag}{}", encode_base62_strict(&entry.value.object_id())),
                "Version":entry.version,"DirVersion":entry.directory_version,"WriteRole":role(entry.write_role)?,
                "NameMac":entry.name_mac,"NameBox":{"T":0,"f0":{"Nonce":encode_base62_strict(&entry.name_box.nonce),"Ciphertext":STANDARD.encode(&entry.name_box.ciphertext)}},
                "DirStatus":entry.directory_status as u8,"BindingMac":entry.binding_mac,"Ctime":entry.creation_time})
        }
    };
    Ok(
        json!({"De":de,"V":variable,"Read":{"Role":role(key.role)?,"Gen":key.generation},"Write":role(write)?}),
    )
}
