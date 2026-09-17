package main

import (
	cryptorand "crypto/rand"
	"crypto/sha256"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"sync"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/merkle"
	teamlib "github.com/foks-proj/go-foks/lib/team"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
)

type deterministicFixtureReader struct {
	counter uint64
	pending []byte
}

const deterministicFixtureTime = proto.Time(1_700_000_000_019)

var deterministicFixtureMu sync.Mutex

func (r *deterministicFixtureReader) Read(dst []byte) (int, error) {
	written := 0
	for written < len(dst) {
		if len(r.pending) == 0 {
			var input [40]byte
			copy(input[:32], []byte("foks-v0.1.9-mutation-fixture-v1"))
			binary.BigEndian.PutUint64(input[32:], r.counter)
			r.counter++
			block := sha256.Sum256(input[:])
			r.pending = block[:]
		}
		count := copy(dst[written:], r.pending)
		written += count
		r.pending = r.pending[count:]
	}
	return written, nil
}

func decodeFixture(path string, out core.Codecable) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return core.DecodeFromBytes(out, data)
}

func retimeUserGroupLink(link *proto.LinkOuter, signers []core.Signer) (*proto.LinkOuter, error) {
	change, _, err := core.OpenGroupChange(link)
	if err != nil {
		return nil, err
	}
	change.Chainer.Base.Time = deterministicFixtureTime
	inner := proto.NewLinkInnerWithGroupChange(*change)
	encoded, err := core.EncodeToBytes(&inner)
	if err != nil {
		return nil, err
	}
	outer := proto.LinkOuterV1{Inner: encoded}
	if err := core.SignStacked(&outer, signers); err != nil {
		return nil, err
	}
	ret := proto.NewLinkOuterWithV1(outer)
	return &ret, nil
}

func retimeTeamEldestLink(
	link *proto.LinkOuter,
	hepks proto.HEPKSet,
	host proto.HostID,
	signers []core.Signer,
) (*proto.LinkOuter, error) {
	set, err := core.ImportHEPKSet(&hepks)
	if err != nil {
		return nil, err
	}
	opened, err := teamlib.OpenEldestLink(link, set, host)
	if err != nil {
		return nil, err
	}
	change := opened.Gc
	change.Chainer.Base.Time = deterministicFixtureTime
	inner := proto.NewLinkInnerWithGroupChange(change)
	encoded, err := inner.EncodeTyped(core.EncoderFactory{})
	if err != nil {
		return nil, err
	}
	outer := proto.LinkOuterV1{Inner: *encoded}
	if err := core.SignStacked(&outer, signers); err != nil {
		return nil, err
	}
	ret := proto.NewLinkOuterWithV1(outer)
	return &ret, nil
}

// validateAdHocCreateSemantics runs the stateless parts of the official
// server's CreateTeamAdHoc path and then checks the cross-object bindings the
// transaction layer relies on when inserting the team and membership links.
func validateAdHocCreateSemantics(
	arg rem.CreateTeamCommonArg,
	uid proto.UID,
	host proto.HostID,
	ownerPuk core.SharedPrivateSuiter,
	device core.PrivateSuiter,
) error {
	hepks, err := core.ImportHEPKSet(&arg.Eta.Obd.Hepks)
	if err != nil {
		return err
	}
	opened, err := teamlib.OpenEldestLink(&arg.Eta.Link, hepks, host)
	if err != nil {
		return err
	}
	teamID, err := opened.Gc.Entity.Entity.ToTeamID()
	if err != nil {
		return err
	}
	if !teamID.Type().IsAdHocTeam() || !opened.Gc.Entity.Host.Eq(host) {
		return fmt.Errorf("ad-hoc eldest has the wrong team type or host")
	}
	ownerID, err := ownerPuk.RollingEntityID()
	if err != nil {
		return err
	}
	if !opened.Gc.Signer.Key.RollingEq(ownerID) || opened.Gc.Signer.KeyOwner == nil {
		return fmt.Errorf("ad-hoc eldest is not signed by the expected owner PUK")
	}
	keyOwner := opened.Gc.Signer.KeyOwner
	if !keyOwner.Party.EntityID().Eq(uid.EntityID()) || !keyOwner.SrcRole.SimpleEq(proto.OwnerRole) {
		return fmt.Errorf("ad-hoc eldest has the wrong owner binding")
	}
	if len(opened.Gc.Changes) != 1 {
		return fmt.Errorf("single-owner ad-hoc eldest has %d member changes", len(opened.Gc.Changes))
	}
	member := opened.Gc.Changes[0]
	if !member.Member.Id.Entity.Eq(uid.EntityID()) || member.Member.Id.Host != nil ||
		!member.Member.SrcRole.SimpleEq(proto.OwnerRole) || !member.DstRole.SimpleEq(proto.OwnerRole) {
		return fmt.Errorf("ad-hoc eldest has the wrong local-owner member")
	}
	if len(opened.Gc.SharedKeys) != len(teamlib.EldestRoles()) ||
		len(arg.Eta.Obd.PtkBoxes.Boxes) != len(opened.Gc.SharedKeys) ||
		len(arg.Eta.Obd.Hepks.V) != len(opened.Gc.SharedKeys) {
		return fmt.Errorf("ad-hoc eldest PTK, HEPK, and box counts differ")
	}
	for index, role := range teamlib.EldestRoles() {
		shared := opened.Gc.SharedKeys[index]
		box := arg.Eta.Obd.PtkBoxes.Boxes[index]
		if shared.Gen != proto.FirstGeneration || box.Gen != proto.FirstGeneration ||
			!shared.Role.SimpleEq(role) || !box.Role.SimpleEq(role) {
			return fmt.Errorf("ad-hoc eldest PTK or box role schedule differs at index %d", index)
		}
	}
	if err := core.VerifyTreeLocationCommitment(
		arg.Eta.NextTreeLocation,
		opened.Gc.Chainer.NextLocationCommitment,
	); err != nil {
		return err
	}
	if err := core.VerifyTreeLocationCommitment(arg.SubchainTreeLocationSeed, opened.Stltc); err != nil {
		return err
	}

	membership, err := core.OpenAndVerifyGenericLink(arg.TeamMembershipLink.Link)
	if err != nil {
		return err
	}
	deviceID, err := device.EntityID()
	if err != nil {
		return err
	}
	if !membership.Link.Entity.Entity.Eq(uid.EntityID()) ||
		!membership.Link.Entity.Host.Eq(host) ||
		!membership.Link.Signer.Entity.Eq(deviceID) || membership.Link.Signer.Host != nil ||
		membership.Link.Chainer.Base.Seqno != proto.ChainEldestSeqno ||
		membership.Link.Chainer.Base.Prev != nil ||
		membership.Link.Chainer.Base.Root != opened.Gc.Chainer.Base.Root {
		return fmt.Errorf("ad-hoc membership has the wrong creator chain binding")
	}
	payloadType, err := membership.Link.Payload.GetT()
	if err != nil {
		return err
	}
	if payloadType != proto.ChainType_TeamMembership {
		return fmt.Errorf("ad-hoc membership has the wrong generic payload type")
	}
	membershipLink := membership.Link.Payload.Teammembership()
	if !membershipLink.Team.Eq(proto.FQTeam{Team: teamID, Host: host}) ||
		!membershipLink.SrcRole.SimpleEq(proto.OwnerRole) {
		return fmt.Errorf("ad-hoc membership targets the wrong team or source role")
	}
	stateType, err := membershipLink.State.GetT()
	if err != nil {
		return err
	}
	if stateType != proto.TeamMembershipLinkState_ApprovedAdHoc {
		return fmt.Errorf("ad-hoc membership is not ApprovedAdHoc")
	}
	destination := membershipLink.State.Approvedadhoc()
	if !destination.Role.SimpleEq(proto.OwnerRole) || destination.Seqno != proto.ChainEldestSeqno {
		return fmt.Errorf("ad-hoc membership has the wrong destination role or sequence")
	}
	return core.VerifyTreeLocationCommitment(
		arg.TeamMembershipLink.NextTreeLocation,
		membership.Link.Chainer.NextLocationCommitment,
	)
}

// writeMutationFixtures encodes provision/revoke RPCs with the official Go
// v0.1.9 generated types while reusing the already verified user-chain
// objects. The arguments are encoding fixtures, not a live server transcript.
func writeMutationFixtures(output, userDir string) error {
	deterministicFixtureMu.Lock()
	defer deterministicFixtureMu.Unlock()
	previousRandom := cryptorand.Reader
	cryptorand.Reader = &deterministicFixtureReader{}
	defer func() { cryptorand.Reader = previousRandom }()

	if err := os.MkdirAll(output, 0o755); err != nil {
		return err
	}
	var chain rem.UserChain
	if err := decodeFixture(filepath.Join(userDir, "user-chain.snowp"), &chain); err != nil {
		return err
	}
	var provision, revoke proto.LinkOuter
	if err := decodeFixture(filepath.Join(userDir, "user-provision-link.snowp"), &provision); err != nil {
		return err
	}
	if err := decodeFixture(filepath.Join(userDir, "user-revoke-link.snowp"), &revoke); err != nil {
		return err
	}
	var uid proto.UID
	if err := decodeFixture(filepath.Join(userDir, "uid.snowp"), &uid); err != nil {
		return err
	}
	revokeChange, _, err := core.OpenGroupChange(&revoke)
	if err != nil {
		return err
	}
	deviceSeedBytes, err := os.ReadFile(filepath.Join(userDir, "device-seed.bin"))
	if err != nil {
		return err
	}
	var deviceSeed proto.SecretSeed32
	if len(deviceSeedBytes) != len(deviceSeed) {
		return core.BadArgsError("device seed fixture has the wrong length")
	}
	copy(deviceSeed[:], deviceSeedBytes)
	device, err := core.NewPrivateSuite25519(
		proto.EntityType_Device,
		proto.NewRoleDefault(proto.RoleType_OWNER),
		deviceSeed,
		revokeChange.Entity.Host,
	)
	if err != nil {
		return err
	}
	var rotationSeed proto.SecretSeed32
	for i := range rotationSeed {
		rotationSeed[i] = byte(i + 201)
	}
	rotationPuk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_User,
		proto.NewRoleDefault(proto.RoleType_OWNER),
		rotationSeed,
		proto.FirstGeneration+2,
		revokeChange.Entity.Host,
	)
	if err != nil {
		return err
	}
	revokeHash, err := core.LinkHash(&revoke)
	if err != nil {
		return err
	}
	treeRoot, err := merkle.ToTreeRoot(&chain.Merkle.Root)
	if err != nil {
		return err
	}
	rotation, err := core.MakeRevokeLink(
		uid,
		revokeChange.Entity.Host,
		device,
		nil,
		[]core.SharedPrivateSuiter{rotationPuk},
		revokeChange.Chainer.Base.Seqno+1,
		*revokeHash,
		*treeRoot,
	)
	if err != nil {
		return err
	}
	rotation.Link, err = retimeUserGroupLink(
		rotation.Link,
		[]core.Signer{rotationPuk, device},
	)
	if err != nil {
		return err
	}
	var parcel proto.SharedKeyParcel
	if err := decodeFixture(filepath.Join(userDir, "puk-parcel.snowp"), &parcel); err != nil {
		return err
	}
	boxSet := proto.SharedKeyBoxSet{
		Id:              parcel.BoxId,
		Boxes:           []proto.SharedKeyBox{parcel.Box},
		TempDHKeySigned: parcel.TempDHKeySigned,
	}
	var selfToken proto.PermissionToken
	for i := range selfToken {
		selfToken[i] = byte(201 + i)
	}
	provisionArg := rem.ProvisionDeviceArg{
		Link:             provision,
		PukBoxes:         boxSet,
		Dlnc:             chain.DeviceNames[1],
		NextTreeLocation: chain.Locations[1],
		SelfToken:        selfToken,
		Hepks:            chain.Hepks,
	}
	provisionFrame, err := rpcRequestFrame(rem.UserProtocolID, 6, provisionArg.Export())
	if err != nil {
		return err
	}
	revokeArg := rem.RevokeDeviceArg{
		Link:             revoke,
		PukBoxes:         boxSet,
		SeedChain:        parcel.SeedChain,
		NextTreeLocation: chain.Locations[2],
		Hepks:            chain.Hepks,
	}
	revokeFrame, err := rpcRequestFrame(rem.UserProtocolID, 7, revokeArg.Export())
	if err != nil {
		return err
	}
	rotationArg := rem.RevokeDeviceArg{
		Link:             *rotation.Link,
		PukBoxes:         boxSet,
		SeedChain:        parcel.SeedChain,
		NextTreeLocation: *rotation.NextTreeLocation,
		Hepks:            *rotation.HEPKSet,
	}
	rotationFrame, err := rpcRequestFrame(rem.UserProtocolID, 7, rotationArg.Export())
	if err != nil {
		return err
	}

	// Build the smallest complete ad-hoc team mutation: one local owner, four
	// generation-one PTKs, and the creator's ApprovedAdHoc membership link.
	pukSeedBytes, err := os.ReadFile(filepath.Join(userDir, "puk-seed.bin"))
	if err != nil {
		return err
	}
	var ownerPukSeed proto.SecretSeed32
	if len(pukSeedBytes) != len(ownerPukSeed) {
		return core.BadArgsError("PUK seed fixture has the wrong length")
	}
	copy(ownerPukSeed[:], pukSeedBytes)
	ownerRole := proto.NewRoleDefault(proto.RoleType_OWNER)
	ownerPuk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_User,
		ownerRole,
		ownerPukSeed,
		proto.FirstGeneration+1,
		revokeChange.Entity.Host,
	)
	if err != nil {
		return err
	}
	teamRoles := teamlib.EldestRoles()
	teamKeys := make([]core.SharedPrivateSuiter, 0, len(teamRoles))
	teamSeeds := make([]proto.SecretSeed32, 0, len(teamRoles))
	teamHepks := proto.HEPKSet{}
	for roleIndex, role := range teamRoles {
		var seed proto.SecretSeed32
		for i := range seed {
			seed[i] = byte(41*roleIndex + i + 11)
		}
		ptk, err := core.NewSharedPrivateSuite25519(
			proto.EntityType_NamedTeam,
			role,
			seed,
			proto.FirstGeneration,
			revokeChange.Entity.Host,
		)
		if err != nil {
			return err
		}
		hepk, err := ptk.ExportHEPK()
		if err != nil {
			return err
		}
		teamKeys = append(teamKeys, ptk)
		teamSeeds = append(teamSeeds, seed)
		teamHepks.V = append(teamHepks.V, *hepk)
	}
	teamBoxesBuilder, err := core.NewSharedKeyBoxer(revokeChange.Entity.Host, ownerPuk)
	if err != nil {
		return err
	}
	ownerPublic, err := core.PublicizeToSPSBoxer(
		ownerPuk,
		proto.FQUser{Uid: uid, HostID: revokeChange.Entity.Host}.FQParty(),
	)
	if err != nil {
		return err
	}
	for _, ptk := range teamKeys {
		if err := teamBoxesBuilder.Box(ptk, ownerPublic); err != nil {
			return err
		}
	}
	teamBoxes, err := teamBoxesBuilder.Finish()
	if err != nil {
		return err
	}
	ownerKey := proto.KeyOwner{Party: uid.ToPartyID(), SrcRole: ownerRole}
	teamEldest, err := teamlib.MakeEldestLink(
		revokeChange.Entity.Host,
		nil,
		ownerKey,
		ownerPuk,
		teamKeys,
		*treeRoot,
		nil,
		nil,
	)
	if err != nil {
		return err
	}
	teamSigners := make([]core.Signer, 0, len(teamKeys)+1)
	for _, key := range teamKeys {
		teamSigners = append(teamSigners, key)
	}
	teamSigners = append(teamSigners, ownerPuk)
	teamEldest.Link, err = retimeTeamEldestLink(
		teamEldest.Link,
		teamHepks,
		revokeChange.Entity.Host,
		teamSigners,
	)
	if err != nil {
		return err
	}
	teamID, err := teamEldest.TeamID.ToTeamID()
	if err != nil {
		return err
	}
	membershipPayload := proto.NewGenericLinkPayloadWithTeammembership(proto.TeamMembershipLink{
		Team:    proto.FQTeam{Team: teamID, Host: revokeChange.Entity.Host},
		SrcRole: ownerRole,
		State: proto.NewTeamMembershipDetailsWithApprovedadhoc(proto.RoleAndSeqno{
			Role:  ownerRole,
			Seqno: proto.ChainEldestSeqno,
		}),
	})
	membership, err := core.MakeGenericLink(
		uid.EntityID(),
		revokeChange.Entity.Host,
		device,
		membershipPayload,
		proto.ChainEldestSeqno,
		nil,
		*treeRoot,
	)
	if err != nil {
		return err
	}
	teamCreateArg := rem.CreateTeamAdHocArg{Carg: rem.CreateTeamCommonArg{
		SubchainTreeLocationSeed: *teamEldest.SubchainTreeLocationSeed,
		Eta: rem.EditTeamArg{
			Link:             *teamEldest.Link,
			NextTreeLocation: *teamEldest.NextTreeLocation,
			Obd: rem.OffchainBoxData{
				PtkBoxes: *teamBoxes,
				Hepks:    teamHepks,
			},
		},
		TeamMembershipLink: rem.PostGenericLinkArg{
			Link:             *membership.Link,
			NextTreeLocation: *membership.NextTreeLocation,
		},
	}}
	if err := validateAdHocCreateSemantics(
		teamCreateArg.Carg,
		uid,
		revokeChange.Entity.Host,
		ownerPuk,
		device,
	); err != nil {
		return fmt.Errorf("validate ad-hoc server semantics: %w", err)
	}
	teamCreateFrame, err := rpcRequestFrame(rem.TeamAdminProtocolID, 15, teamCreateArg.Export())
	if err != nil {
		return err
	}

	// Build the equivalent single-owner named-team mutation. Named creation
	// adds a reservation/name commitment, one removal-key commitment and dual
	// boxes, and an Approved (rather than ApprovedAdHoc) membership link.
	namedNameUtf8 := proto.NameUtf8("AuditTeam")
	namedName := proto.Name("auditteam")
	var reserveToken proto.ReservationToken
	for i := range reserveToken {
		reserveToken[i] = byte(101 + i)
	}
	namedReservation := rem.ReserveNameRes{
		Tok:   reserveToken,
		Seq:   7,
		Etime: deterministicFixtureTime + 1_000_000,
	}
	removalKey, err := teamlib.NewTeamRemovalKey()
	if err != nil {
		return err
	}
	removalCommitment, err := core.ComputeKeyCommitment(removalKey)
	if err != nil {
		return err
	}
	namedEldest, err := teamlib.MakeEldestLink(
		revokeChange.Entity.Host,
		&rem.NameCommitment{Name: namedName, Seq: namedReservation.Seq},
		ownerKey,
		ownerPuk,
		teamKeys,
		*treeRoot,
		removalCommitment,
		nil,
	)
	if err != nil {
		return err
	}
	namedEldest.Link, err = retimeTeamEldestLink(
		namedEldest.Link,
		teamHepks,
		revokeChange.Entity.Host,
		teamSigners,
	)
	if err != nil {
		return err
	}
	namedTeamID, err := namedEldest.TeamID.ToTeamID()
	if err != nil {
		return err
	}
	adminPublic, err := core.PublicizeToSPSBoxer(
		teamKeys[2],
		proto.FQTeam{Team: namedTeamID, Host: revokeChange.Entity.Host}.FQParty(),
	)
	if err != nil {
		return err
	}
	removalMetadata := rem.TeamRemovalKeyMetadata{
		Tm: proto.FQTeam{Team: namedTeamID, Host: revokeChange.Entity.Host},
		Member: proto.FQUser{
			Uid:    uid,
			HostID: revokeChange.Entity.Host,
		}.FQParty(),
		SrcRole: ownerRole,
		Dst:     proto.RoleAndSeqno{Role: ownerRole, Seqno: proto.ChainEldestSeqno},
	}
	removalBoxes, err := teamlib.BoxTeamRemovalKey(
		ownerPuk,
		adminPublic,
		ownerPublic,
		removalMetadata,
		removalKey,
	)
	if err != nil {
		return err
	}
	namedMembershipPayload := proto.NewGenericLinkPayloadWithTeammembership(proto.TeamMembershipLink{
		Team:    proto.FQTeam{Team: namedTeamID, Host: revokeChange.Entity.Host},
		SrcRole: ownerRole,
		State: proto.NewTeamMembershipDetailsWithApproved(proto.TeamMembershipApprovedDetails{
			Dst:     proto.RoleAndSeqno{Role: ownerRole, Seqno: proto.ChainEldestSeqno},
			KeyComm: removalBoxes.Comm,
		}),
	})
	namedMembership, err := core.MakeGenericLink(
		uid.EntityID(),
		revokeChange.Entity.Host,
		device,
		namedMembershipPayload,
		proto.ChainEldestSeqno,
		nil,
		*treeRoot,
	)
	if err != nil {
		return err
	}
	namedCreateArg := rem.CreateTeamArg{
		NameUtf8:                 namedNameUtf8,
		TeamnameCommitmentKey:    *namedEldest.TeamnameCommitmentKey,
		SubchainTreeLocationSeed: *namedEldest.SubchainTreeLocationSeed,
		Rnr:                      namedReservation,
		Eta: rem.EditTeamArg{
			Link:             *namedEldest.Link,
			NextTreeLocation: *namedEldest.NextTreeLocation,
			Obd: rem.OffchainBoxData{
				PtkBoxes:    *teamBoxes,
				RemovalKeys: []rem.TeamRemovalBoxData{*removalBoxes},
				Hepks:       teamHepks,
			},
		},
		TeamMembershipLink: rem.PostGenericLinkArg{
			Link:             *namedMembership.Link,
			NextTreeLocation: *namedMembership.NextTreeLocation,
		},
	}
	namedCreateFrame, err := rpcRequestFrame(rem.TeamAdminProtocolID, 1, namedCreateArg.Export())
	if err != nil {
		return err
	}

	// Add one local user at the default member role. This is the exact
	// open-viewership TeamAdmin.editTeam path: existing visible PTKs are boxed
	// to the target PUK, while the signed link introduces no new PTK.
	var targetPukSeed proto.SecretSeed32
	for i := range targetPukSeed {
		targetPukSeed[i] = byte(0xa5 ^ i)
	}
	targetPuk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_User,
		ownerRole,
		targetPukSeed,
		proto.FirstGeneration+2,
		revokeChange.Entity.Host,
	)
	if err != nil {
		return err
	}
	targetRolling, err := targetPuk.RollingEntityID()
	if err != nil {
		return err
	}
	targetEntity, err := targetRolling.Persistent(proto.PartyType_User)
	if err != nil {
		return err
	}
	targetUID, err := targetEntity.ToUID()
	if err != nil {
		return err
	}
	targetPublic, err := core.PublicizeToSPSBoxer(
		targetPuk,
		proto.FQUser{Uid: targetUID, HostID: revokeChange.Entity.Host}.FQParty(),
	)
	if err != nil {
		return err
	}
	targetMember, targetHepk, err := targetPublic.ExportToMember(revokeChange.Entity.Host)
	if err != nil {
		return err
	}
	targetMember.SrcRole = ownerRole
	memberRole := proto.MemberRole{DstRole: proto.DefaultRole, Member: *targetMember}
	addRemovalKey, err := teamlib.NewTeamRemovalKey()
	if err != nil {
		return err
	}
	addRemovalCommitment, err := core.ComputeKeyCommitment(addRemovalKey)
	if err != nil {
		return err
	}
	if err := memberRole.Member.AddRemovalKeyCommitment(addRemovalCommitment); err != nil {
		return err
	}
	namedHash, err := core.LinkHash(namedEldest.Link)
	if err != nil {
		return err
	}
	addLink, err := teamlib.MakeTeamLink(
		revokeChange.Entity.Host,
		namedTeamID,
		ownerKey,
		ownerPuk,
		[]proto.MemberRole{memberRole},
		nil,
		proto.ChainEldestSeqno+1,
		*namedHash,
		*treeRoot,
		nil,
	)
	if err != nil {
		return err
	}
	addLink.Link, err = retimeUserGroupLink(addLink.Link, []core.Signer{ownerPuk})
	if err != nil {
		return err
	}
	addBoxBuilder, err := core.NewSharedKeyBoxer(revokeChange.Entity.Host, ownerPuk)
	if err != nil {
		return err
	}
	for index := range teamRoles {
		if index < 2 {
			if err := addBoxBuilder.Box(teamKeys[index], targetPublic); err != nil {
				return err
			}
		}
	}
	addPtkBoxes, err := addBoxBuilder.Finish()
	if err != nil {
		return err
	}
	addRemovalMetadata := rem.TeamRemovalKeyMetadata{
		Tm:      proto.FQTeam{Team: namedTeamID, Host: revokeChange.Entity.Host},
		Member:  proto.FQUser{Uid: targetUID, HostID: revokeChange.Entity.Host}.FQParty(),
		SrcRole: ownerRole,
		Dst: proto.RoleAndSeqno{
			Role:  proto.DefaultRole,
			Seqno: proto.ChainEldestSeqno + 1,
		},
	}
	addRemovalBoxes, err := teamlib.BoxTeamRemovalKey(
		ownerPuk,
		adminPublic,
		targetPublic,
		addRemovalMetadata,
		addRemovalKey,
	)
	if err != nil {
		return err
	}
	if addRemovalBoxes.Comm != *addRemovalCommitment {
		return fmt.Errorf("addition removal box commitment differs from signed member commitment")
	}
	additionHepks := proto.HEPKSet{V: []proto.HEPK{*targetHepk}}
	allAdditionHepks := proto.HEPKSet{V: append(append([]proto.HEPK{}, teamHepks.V...), *targetHepk)}
	eldestSet, err := core.ImportHEPKSet(&teamHepks)
	if err != nil {
		return err
	}
	openedEldest, err := teamlib.OpenEldestLink(
		namedEldest.Link,
		eldestSet,
		revokeChange.Entity.Host,
	)
	if err != nil {
		return err
	}
	additionSet, err := core.ImportHEPKSet(&allAdditionHepks)
	if err != nil {
		return err
	}
	openedAddition, err := teamlib.OpenTeamLink(
		addLink.Link,
		additionSet,
		&namedTeamID,
		revokeChange.Entity.Host,
		openedEldest.RosterPost,
	)
	if err != nil {
		return fmt.Errorf("official addition link verification: %w", err)
	}
	if len(openedAddition.Sched.Additions) != 1 || len(openedAddition.SharedKeys) != 0 {
		return fmt.Errorf("addition produced the wrong roster or PTK schedule")
	}
	addArgument := rem.EditTeamArg{
		Link:             *addLink.Link,
		NextTreeLocation: *addLink.NextTreeLocation,
		Obd: rem.OffchainBoxData{
			PtkBoxes:    *addPtkBoxes,
			RemovalKeys: []rem.TeamRemovalBoxData{*addRemovalBoxes},
			Hepks:       additionHepks,
		},
		InsLocalPermsFor: []proto.PartyID{targetUID.ToPartyID()},
	}
	addFrame, err := rpcRequestFrame(rem.TeamAdminProtocolID, 2, addArgument.Export())
	if err != nil {
		return err
	}
	addResult := rem.EditTeamRes{}
	addHash, err := core.LinkHash(addLink.Link)
	if err != nil {
		return err
	}

	// Build an alternative post-addition demotion. Moving the member from the
	// default member role to member-min rotates only the old role's PTK.
	mutationRandom := cryptorand.Reader
	cryptorand.Reader = &deterministicFixtureReader{counter: 10_000}
	memberMinRole := teamRoles[0]
	demotedMember := *targetMember
	demoteRole := proto.MemberRole{DstRole: memberMinRole, Member: demotedMember}
	var demoteSeed proto.SecretSeed32
	for i := range demoteSeed {
		demoteSeed[i] = byte(0x91 - i)
	}
	demotePtk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_NamedTeam,
		teamRoles[1],
		demoteSeed,
		proto.FirstGeneration+1,
		revokeChange.Entity.Host,
	)
	if err != nil {
		return err
	}
	demoteLink, err := teamlib.MakeTeamLink(
		revokeChange.Entity.Host,
		namedTeamID,
		ownerKey,
		ownerPuk,
		[]proto.MemberRole{demoteRole},
		[]core.SharedPrivateSuiter{demotePtk},
		proto.ChainEldestSeqno+2,
		*addHash,
		*treeRoot,
		nil,
	)
	if err != nil {
		return err
	}
	demoteLink.Link, err = retimeUserGroupLink(demoteLink.Link, []core.Signer{demotePtk, ownerPuk})
	if err != nil {
		return err
	}
	demoteHepk, err := demotePtk.ExportHEPK()
	if err != nil {
		return err
	}
	demoteSet, err := core.ImportHEPKSet(&proto.HEPKSet{V: append(
		append([]proto.HEPK{}, allAdditionHepks.V...), *demoteHepk,
	)})
	if err != nil {
		return err
	}
	openedDemotion, err := teamlib.OpenTeamLink(
		demoteLink.Link,
		demoteSet,
		&namedTeamID,
		revokeChange.Entity.Host,
		openedAddition.RosterPost,
	)
	if err != nil {
		return fmt.Errorf("official demotion link verification: %w", err)
	}
	if len(openedDemotion.Sched.Items) == 0 || len(openedDemotion.SharedKeys) != 1 {
		return fmt.Errorf("demotion produced the wrong roster or PTK schedule")
	}
	cryptorand.Reader = mutationRandom

	// Remove the member just added. A removal from the default member role
	// rotates the member-min and member PTKs, boxes their new generations only
	// to the remaining owner, and chains each old generation under the new one.
	noneRole := proto.NewRoleDefault(proto.RoleType_NONE)
	removedMember := *targetMember
	removedMember.Keys = proto.NewMemberKeysWithNone()
	removeRole := proto.MemberRole{DstRole: noneRole, Member: removedMember}
	rotatedKeys := make([]core.SharedPrivateSuiter, 0, 2)
	rotatedSeeds := make([]proto.SecretSeed32, 0, 2)
	rotationHepks := proto.HEPKSet{}
	seedChain := make([]proto.SeedChainBox, 0, 2)
	rotateBoxBuilder, err := core.NewSharedKeyBoxer(revokeChange.Entity.Host, ownerPuk)
	if err != nil {
		return err
	}
	for index := 0; index < 2; index++ {
		var seed proto.SecretSeed32
		for i := range seed {
			seed[i] = byte(0xd3 - 17*index - i)
		}
		rotated, err := core.NewSharedPrivateSuite25519(
			proto.EntityType_NamedTeam,
			teamRoles[index],
			seed,
			proto.FirstGeneration+1,
			revokeChange.Entity.Host,
		)
		if err != nil {
			return err
		}
		hepk, err := rotated.ExportHEPK()
		if err != nil {
			return err
		}
		oldSeed := teamKeys[index].ExportToBoxCleartext(
			proto.FQUser{Uid: uid, HostID: revokeChange.Entity.Host}.FQParty().FQEntity(),
		)
		secretBoxKey := rotated.SecretBoxKey()
		oldBox, err := core.SealIntoSecretBox(&oldSeed, &secretBoxKey)
		if err != nil {
			return err
		}
		seedChain = append(seedChain, proto.SeedChainBox{
			Box:  *oldBox,
			Gen:  proto.FirstGeneration,
			Role: teamRoles[index],
		})
		if err := rotateBoxBuilder.Box(rotated, ownerPublic); err != nil {
			return err
		}
		rotatedKeys = append(rotatedKeys, rotated)
		rotatedSeeds = append(rotatedSeeds, seed)
		rotationHepks.V = append(rotationHepks.V, *hepk)
	}
	rotatedBoxes, err := rotateBoxBuilder.Finish()
	if err != nil {
		return err
	}
	removeLink, err := teamlib.MakeTeamLink(
		revokeChange.Entity.Host,
		namedTeamID,
		ownerKey,
		ownerPuk,
		[]proto.MemberRole{removeRole},
		rotatedKeys,
		proto.ChainEldestSeqno+2,
		*addHash,
		*treeRoot,
		nil,
	)
	if err != nil {
		return err
	}
	rotationSigners := []core.Signer{rotatedKeys[0], rotatedKeys[1], ownerPuk}
	removeLink.Link, err = retimeUserGroupLink(removeLink.Link, rotationSigners)
	if err != nil {
		return err
	}
	removePayload := rem.TeamRemovalMACPayload{
		Team:    proto.FQTeam{Team: namedTeamID, Host: revokeChange.Entity.Host},
		Member:  proto.FQUser{Uid: targetUID, HostID: revokeChange.Entity.Host}.FQParty(),
		SrcRole: ownerRole,
		Admin:   proto.FQUser{Uid: uid, HostID: revokeChange.Entity.Host}.FQParty(),
		Root:    *treeRoot,
		Tm:      deterministicFixtureTime,
	}
	removalHmacKey := proto.HMACKey(*addRemovalKey)
	removeMAC, err := core.Hmac(&removePayload, &removalHmacKey)
	if err != nil {
		return err
	}
	removeProof := rem.TeamRemovalAndComm{
		Rm:   rem.TeamRemoval{Mac: *removeMAC, Payload: removePayload},
		Comm: *addRemovalCommitment,
	}
	allRotationHepks := proto.HEPKSet{V: append(append([]proto.HEPK{}, allAdditionHepks.V...), rotationHepks.V...)}
	rotationSet, err := core.ImportHEPKSet(&allRotationHepks)
	if err != nil {
		return err
	}
	openedRemoval, err := teamlib.OpenTeamLink(
		removeLink.Link,
		rotationSet,
		&namedTeamID,
		revokeChange.Entity.Host,
		openedAddition.RosterPost,
	)
	if err != nil {
		return fmt.Errorf("official removal link verification: %w", err)
	}
	if len(openedRemoval.Sched.Removals) != 1 || len(openedRemoval.SharedKeys) != 2 {
		return fmt.Errorf("removal produced the wrong roster or PTK schedule")
	}
	removeArgument := rem.EditTeamArg{
		Link:             *removeLink.Link,
		NextTreeLocation: *removeLink.NextTreeLocation,
		Obd: rem.OffchainBoxData{
			PtkBoxes:  *rotatedBoxes,
			SeedChain: seedChain,
			Removals:  []rem.TeamRemovalAndComm{removeProof},
			Hepks:     rotationHepks,
		},
	}
	removeFrame, err := rpcRequestFrame(rem.TeamAdminProtocolID, 2, removeArgument.Export())
	if err != nil {
		return err
	}
	var adminBearer rem.TeamBearerToken
	for i := range adminBearer {
		adminBearer[i] = byte(0x70 + i)
	}
	bearerMakeArg := rem.MakeInertTeamBearerTokenArg{
		Team: namedTeamID,
		Role: proto.OwnerRole,
		Gen:  proto.FirstGeneration,
	}
	bearerMakeFrame, err := rpcRequestFrame(rem.TeamAdminProtocolID, 3, bearerMakeArg.Export())
	if err != nil {
		return err
	}
	bearerPayload := rem.TeamBearerTokenChallengePayload{
		User: proto.FQUser{Uid: uid, HostID: revokeChange.Entity.Host},
		Team: namedTeamID,
		Role: proto.OwnerRole,
		Gen:  proto.FirstGeneration,
		Tok:  adminBearer,
		Tm:   deterministicFixtureTime,
	}
	bearerSig, bearerBlob, err := core.Sign2(teamKeys[3], &bearerPayload)
	if err != nil {
		return err
	}
	bearerActivateArg := rem.ActivateTeamBearerTokenArg{Bl: *bearerBlob, Sig: *bearerSig}
	bearerActivateFrame, err := rpcRequestFrame(rem.TeamAdminProtocolID, 4, bearerActivateArg.Export())
	if err != nil {
		return err
	}
	loadRemovalArg := rem.LoadRemovalKeyBoxForTeamAdminArg{
		Tok: adminBearer,
		Member: proto.FQUser{
			Uid: targetUID, HostID: revokeChange.Entity.Host,
		}.FQParty(),
		SrcRole: ownerRole,
	}
	loadRemovalFrame, err := rpcRequestFrame(rem.TeamAdminProtocolID, 10, loadRemovalArg.Export())
	if err != nil {
		return err
	}
	namedReserveFrame, err := rpcRequestFrame(
		rem.TeamAdminProtocolID,
		0,
		(rem.ReserveTeamnameArg{N: namedName}).Export(),
	)
	if err != nil {
		return err
	}
	hostConfigFrame, err := rpcRequestFrame(
		rem.UserProtocolID,
		24,
		(rem.GetHostConfigArg{}).Export(),
	)
	if err != nil {
		return err
	}
	w := writer{dir: output}
	if err := writeBackupFixtures(&w, userDir, &chain); err != nil {
		return err
	}
	hostConfig := proto.HostConfig{
		Metering: proto.Metering{Users: true, VHosts: false, PerVHostDisk: true},
		Viewership: proto.HostViewership{
			User: proto.ViewershipMode_Open,
			Team: proto.ViewershipMode_OpenToAdmin,
		},
		Typ: proto.HostType_Standalone,
		Icr: proto.InviteCodeRegime_CodeOptional,
	}
	if err := w.object("host-config-open.snowp", &hostConfig); err != nil {
		return err
	}
	if err := w.object("box-set.snowp", &boxSet); err != nil {
		return err
	}
	if err := w.object("rotation-link.snowp", rotation.Link); err != nil {
		return err
	}
	if err := w.object("adhoc-team-link.snowp", teamEldest.Link); err != nil {
		return err
	}
	if err := w.object("adhoc-membership-link.snowp", membership.Link); err != nil {
		return err
	}
	if err := w.object("adhoc-team-id.snowp", &teamID); err != nil {
		return err
	}
	if err := w.object("adhoc-box-set.snowp", teamBoxes); err != nil {
		return err
	}
	if err := w.object("named-team-link.snowp", namedEldest.Link); err != nil {
		return err
	}
	if err := w.object("named-membership-link.snowp", namedMembership.Link); err != nil {
		return err
	}
	if err := w.object("named-team-id.snowp", &namedTeamID); err != nil {
		return err
	}
	if err := w.object("named-reservation.snowp", &namedReservation); err != nil {
		return err
	}
	if err := w.object("named-removal-boxes.snowp", removalBoxes); err != nil {
		return err
	}
	if err := w.object("add-member-link.snowp", addLink.Link); err != nil {
		return err
	}
	if err := w.object("add-member-ptk-boxes.snowp", addPtkBoxes); err != nil {
		return err
	}
	if err := w.object("add-member-removal-boxes.snowp", addRemovalBoxes); err != nil {
		return err
	}
	if err := w.object("add-member-edit-result.snowp", &addResult); err != nil {
		return err
	}
	if err := w.object("demote-member-link.snowp", demoteLink.Link); err != nil {
		return err
	}
	if err := w.object("add-member-target-uid.snowp", &targetUID); err != nil {
		return err
	}
	if err := w.object("remove-member-link.snowp", removeLink.Link); err != nil {
		return err
	}
	if err := w.object("remove-member-ptk-boxes.snowp", rotatedBoxes); err != nil {
		return err
	}
	if err := w.object("remove-member-offchain.snowp", &removeArgument.Obd); err != nil {
		return err
	}
	for index := range seedChain {
		if err := w.object(
			[]string{"remove-member-seed-chain-member-min.snowp", "remove-member-seed-chain-member.snowp"}[index],
			&seedChain[index],
		); err != nil {
			return err
		}
	}
	if err := w.object("remove-member-proof.snowp", &removeProof); err != nil {
		return err
	}
	if err := w.object("team-bearer-token.snowp", &adminBearer); err != nil {
		return err
	}
	if err := w.object("team-bearer-challenge-payload.snowp", &bearerPayload); err != nil {
		return err
	}
	if err := w.object("team-bearer-challenge-blob.snowp", bearerBlob); err != nil {
		return err
	}
	if err := w.object("team-bearer-signature.snowp", bearerSig); err != nil {
		return err
	}
	if err := w.object("team-removal-admin-box.snowp", &addRemovalBoxes.Team); err != nil {
		return err
	}
	for _, raw := range []struct {
		name string
		data []byte
	}{
		{"self-token.bin", selfToken[:]},
		{"provision-request.frame", provisionFrame},
		{"revoke-request.frame", revokeFrame},
		{"rotation-next-tree-location.bin", rotation.NextTreeLocation[:]},
		{"rotation-puk-seed.bin", rotationSeed[:]},
		{"rotation-request.frame", rotationFrame},
		{"adhoc-create-request.frame", teamCreateFrame},
		{"adhoc-next-tree-location.bin", teamEldest.NextTreeLocation[:]},
		{"adhoc-subchain-tree-location.bin", teamEldest.SubchainTreeLocationSeed[:]},
		{"adhoc-membership-next-tree-location.bin", membership.NextTreeLocation[:]},
		{"host-config-request.frame", hostConfigFrame},
		{"named-create-request.frame", namedCreateFrame},
		{"named-reserve-request.frame", namedReserveFrame},
		{"named-next-tree-location.bin", namedEldest.NextTreeLocation[:]},
		{"named-subchain-tree-location.bin", namedEldest.SubchainTreeLocationSeed[:]},
		{"named-membership-next-tree-location.bin", namedMembership.NextTreeLocation[:]},
		{"named-team-name-commitment-key.bin", namedEldest.TeamnameCommitmentKey[:]},
		{"named-removal-key.bin", removalKey[:]},
		{"add-member-next-tree-location.bin", addLink.NextTreeLocation[:]},
		{"add-member-removal-key.bin", addRemovalKey[:]},
		{"add-member-target-puk-seed.bin", targetPukSeed[:]},
		{"add-member-request.frame", addFrame},
		{"demote-member-ptk-seed.bin", demoteSeed[:]},
		{"demote-member-next-tree-location.bin", demoteLink.NextTreeLocation[:]},
		{"remove-member-next-tree-location.bin", removeLink.NextTreeLocation[:]},
		{"remove-member-request.frame", removeFrame},
		{"team-bearer-make-request.frame", bearerMakeFrame},
		{"team-bearer-activate-request.frame", bearerActivateFrame},
		{"team-removal-key-load-request.frame", loadRemovalFrame},
	} {
		if err := w.raw(raw.name, raw.data); err != nil {
			return err
		}
	}
	for i, seed := range rotatedSeeds {
		if err := w.raw(
			[]string{"remove-member-ptk-member-min-seed.bin", "remove-member-ptk-member-seed.bin"}[i],
			seed[:],
		); err != nil {
			return err
		}
	}
	for i, seed := range teamSeeds {
		if err := w.raw(
			[]string{
				"adhoc-ptk-member-min-seed.bin",
				"adhoc-ptk-member-seed.bin",
				"adhoc-ptk-admin-seed.bin",
				"adhoc-ptk-owner-seed.bin",
			}[i],
			seed[:],
		); err != nil {
			return err
		}
	}
	sort.Slice(w.files, func(i, j int) bool { return w.files[i].File < w.files[j].File })
	sort.Slice(w.rpcFiles, func(i, j int) bool { return w.rpcFiles[i].File < w.rpcFiles[j].File })
	manifest := struct {
		Format            string    `json:"format"`
		FOKSVersion       string    `json:"foks_version"`
		Generator         string    `json:"generator"`
		GoModuleGenerated bool      `json:"go_module_generated"`
		ServerObserved    bool      `json:"server_observed"`
		Files             []fixture `json:"files"`
		RawFiles          []fixture `json:"raw_files"`
	}{
		Format:            "foks-v0.1.9-user-mutation-fixtures-v3",
		FOKSVersion:       "v0.1.9",
		Generator:         "sha256-counter-v1",
		GoModuleGenerated: true,
		ServerObserved:    false,
		Files:             w.files,
		RawFiles:          w.rpcFiles,
	}
	encoded, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(output, "manifest.json"), append(encoded, '\n'), 0o644)
}
