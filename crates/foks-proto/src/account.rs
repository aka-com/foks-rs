//! Account wire forms. Shape validation is separate from authenticated rename policy.
use crate::{
    array, decode, encode, fixed_blob, option, text, Result, UserLink, UsernameReservation, Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedUsernameFullUpdate {
    pub link: UserLink,
    pub commitment_key: [u8; 16],
    pub reservation: UsernameReservation,
    pub next_tree_location: [u8; 32],
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeUsernameArgument {
    pub username: String,
    pub full: Option<ChangedUsernameFullUpdate>,
}
impl ChangeUsernameArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 2)?;
        let full = option(&f[1], |v| {
            let f = array(v, 4)?;
            Ok(ChangedUsernameFullUpdate {
                link: UserLink::decode(&encode(&f[0])?)?,
                commitment_key: fixed_blob(&f[1], "username commitment key")?,
                reservation: UsernameReservation::decode(&encode(&f[2])?)?,
                next_tree_location: fixed_blob(&f[3], "next tree location")?,
            })
        })?;
        Ok(Self {
            username: text(&f[0])?,
            full,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let full = match &self.full {
            None => Value::Null,
            Some(f) => Value::Array(vec![
                decode(&f.link.encoded()?)?,
                Value::Binary(f.commitment_key.to_vec()),
                f.reservation.to_value(),
                Value::Binary(f.next_tree_location.to_vec()),
            ]),
        };
        Ok(encode(&Value::Array(vec![
            Value::Text(self.username.as_bytes().to_vec()),
            full,
        ]))?)
    }
}
