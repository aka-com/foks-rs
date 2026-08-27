//! Short-lived TeamAdmin bearer tokens and removal-key retrieval.

use foks_crypto::{
    open_team_removal_key_for_member, sign_team_bearer_token_challenge, SharedKeyDecapsulator,
    TeamRemovalKeyExpectation,
};
use foks_proto::{EntityId, Role, SecretSeed, TeamBearerTokenChallenge};
use foks_rpc::{
    decode_team_bearer_token, decode_team_removal_key_box,
    encode_activate_team_bearer_token_request, encode_load_team_removal_key_box_request,
    encode_make_team_bearer_token_request,
};
use foks_verify::VerifiedTeamMemberState;

use super::{AuthenticatedTeamOutcome, TeamPrivateKey};
use crate::{now_microseconds, Error, FoksClient, PinnedHost, Result};

impl FoksClient {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn load_team_removal_key(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        team: &AuthenticatedTeamOutcome,
        member: &VerifiedTeamMemberState,
    ) -> Result<SecretSeed> {
        let (bearer_role, bearer_generation, bearer_seed) = bearer_signer(team)?;
        let inert = self.call_with_material(
            host,
            &host.user,
            &encode_make_team_bearer_token_request(
                team.verified.team(),
                bearer_role,
                bearer_generation,
            )?,
            auth_seed,
            certificate_chain,
        )?;
        let token = decode_team_bearer_token(&inert)?;
        let challenge = TeamBearerTokenChallenge {
            user: uid.clone(),
            user_host: host.host_id().clone(),
            team: team.verified.team().clone(),
            role: bearer_role,
            generation: bearer_generation,
            token,
            time: now_microseconds()?,
        };
        let signature = sign_team_bearer_token_challenge(bearer_seed, &challenge)?;
        self.call_void_with_material(
            host,
            &host.user,
            &encode_activate_team_bearer_token_request(&challenge, &signature)?,
            auth_seed,
            certificate_chain,
        )?;

        let member_host = member.scoped_host.as_ref().unwrap_or(host.host_id());
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_load_team_removal_key_box_request(
                &token,
                &member.party,
                member_host,
                member.source_role,
            )?,
            auth_seed,
            certificate_chain,
        )?;
        let boxed = decode_team_removal_key_box(&response)?;
        let commitment = member.removal_key_commitment.ok_or(Error::TeamBinding(
            "team member has no authenticated removal-key commitment",
        ))?;
        let private = team
            .ptks
            .iter()
            .find(|key| key.role == boxed.role && key.generation == boxed.generation)
            .ok_or(Error::KeyBinding(
                "removal-key box requires an unavailable historical PTK",
            ))?;
        let receiver = SharedKeyDecapsulator::new(&private.seed, team.verified.team().clone())?;
        let (key, _) = open_team_removal_key_for_member(
            &boxed,
            &receiver,
            &TeamRemovalKeyExpectation {
                commitment: &commitment,
                team: team.verified.team(),
                host: host.host_id(),
                member: &member.party,
                member_host,
                source_role: member.source_role,
            },
        )?;
        Ok(key)
    }
}

fn bearer_signer(team: &AuthenticatedTeamOutcome) -> Result<(Role, u64, &SecretSeed)> {
    let public = team
        .verified
        .shared_keys()
        .iter()
        .filter(|key| {
            matches!(
                key.role.kind(),
                foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
            )
        })
        .max_by_key(|key| key.role)
        .ok_or(Error::KeyBinding(
            "team administrator has no current bearer-token PTK",
        ))?;
    let signer: &TeamPrivateKey = team
        .ptks
        .iter()
        .find(|key| key.role == public.role && key.generation == public.generation)
        .ok_or(Error::KeyBinding(
            "current bearer-token PTK secret is unavailable",
        ))?;
    Ok((signer.role, signer.generation, &signer.seed))
}
