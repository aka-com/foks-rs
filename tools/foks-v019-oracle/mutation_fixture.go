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
	hostConfigFrame, err := rpcRequestFrame(
		rem.UserProtocolID,
		24,
		(rem.GetHostConfigArg{}).Export(),
	)
	if err != nil {
		return err
	}
	w := writer{dir: output}
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
	} {
		if err := w.raw(raw.name, raw.data); err != nil {
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
		Format                  string    `json:"format"`
		FOKSVersion             string    `json:"foks_version"`
		Generator               string    `json:"generator"`
		OfficialBuilt           bool      `json:"official_built"`
		ServerSemanticsVerified bool      `json:"server_semantics_verified"`
		Files                   []fixture `json:"files"`
		RawFiles                []fixture `json:"raw_files"`
	}{
		Format:                  "foks-v0.1.9-user-mutation-fixtures-v2",
		FOKSVersion:             "v0.1.9",
		Generator:               "sha256-counter-v1",
		OfficialBuilt:           true,
		ServerSemanticsVerified: true,
		Files:                   w.files,
		RawFiles:                w.rpcFiles,
	}
	encoded, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(output, "manifest.json"), append(encoded, '\n'), 0o644)
}
