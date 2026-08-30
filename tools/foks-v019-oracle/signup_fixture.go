package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"time"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/merkle"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
)

// writeSignupFixtures exercises the complete command-line-only construction
// path used immediately before Reg.signup. It deliberately does not contact a
// server, consume an invite, or create an account.
func writeSignupFixtures(output string, host proto.HostID, root *proto.MerkleRoot) error {
	if err := os.MkdirAll(output, 0o755); err != nil {
		return err
	}
	owner := proto.OwnerRole
	var deviceSeed, pukSeed proto.SecretSeed32
	for i := range deviceSeed {
		deviceSeed[i] = byte(31 + i)
		pukSeed[i] = byte(91 + i)
	}
	device, err := core.NewPrivateSuite25519(proto.EntityType_Device, owner, deviceSeed, host)
	if err != nil {
		return err
	}
	puk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_PUKVerify, owner, pukSeed, proto.FirstGeneration, host,
	)
	if err != nil {
		return err
	}
	devicePublic, err := device.Publicize(&host)
	if err != nil {
		return err
	}
	pukBox, err := core.BoxOne(host, puk, device, devicePublic)
	if err != nil {
		return err
	}
	treeRoot, err := merkle.ToTreeRoot(root)
	if err != nil {
		return err
	}
	username := proto.Name("signupfixture")
	deviceLabel := proto.DeviceLabel{
		DeviceType: proto.DeviceType_Computer,
		Name:       proto.DeviceNameNormalized("signup device"),
		Serial:     proto.FirstDeviceSerial,
	}
	eldest, err := core.MakeEldestLink(
		host,
		rem.NameCommitment{Name: username, Seq: proto.FirstNameSeqno},
		device,
		puk,
		deviceLabel,
		*treeRoot,
		nil,
	)
	if err != nil {
		return err
	}
	hepks, err := core.ImportHEPKSet(eldest.HEPKSet)
	if err != nil {
		return err
	}
	opened, err := core.OpenEldestLink(eldest.Link, hepks, host)
	if err != nil {
		return err
	}
	var unboxed proto.SharedKeySeed
	if len(pukBox.Boxes) != 1 {
		return core.BoxError("expected one initial PUK box")
	}
	if err := core.OpenBoxInSet(
		&unboxed,
		pukBox.Boxes[0].Box,
		pukBox.TempDHKeySigned,
		&pukBox.Id,
		devicePublic,
		device,
	); err != nil {
		return err
	}
	if unboxed.Seed != pukSeed || unboxed.Gen != proto.FirstGeneration || unboxed.Role != owner {
		return core.BoxError("initial PUK box cleartext mismatch")
	}
	var reservationToken proto.ReservationToken
	var selfToken proto.PermissionToken
	for i := range reservationToken {
		reservationToken[i] = byte(151 + i)
		selfToken[i] = byte(181 + i)
	}
	reservation := rem.ReserveNameRes{
		Tok:   reservationToken,
		Seq:   proto.FirstNameSeqno,
		Etime: proto.Time(2_000_000_000_000_000),
	}
	dlnck := rem.DeviceLabelNameAndCommitmentKey{
		Dln: proto.DeviceLabelAndName{
			Label: deviceLabel,
			Nv:    proto.NormalizationVersion_V0,
			Name:  proto.DeviceName("signup device"),
		},
		CommitmentKey: *eldest.DevNameCommitmentKey,
	}
	argument := rem.SignupArg{
		UsernameUtf8:             proto.NameUtf8("signupfixture"),
		Rur:                      reservation,
		Link:                     *eldest.Link,
		PukBox:                   *pukBox,
		UsernameCommitmentKey:    *eldest.UsernameCommitmentKey,
		Dlnck:                    dlnck,
		NextTreeLocation:         *eldest.NextTreeLocation,
		InviteCode:               rem.NewInviteCodeWithEmpty(),
		Email:                    proto.Email("fixture@example.com"),
		SubchainTreeLocationSeed: *eldest.SubchainTreeLocationSeed,
		SelfToken:                selfToken,
		Hepks:                    *eldest.HEPKSet,
		Sso:                      rem.NewRegSSOArgsWithNone(),
	}
	reserveFrame, err := rpcRequestFrameAt(
		rem.RegProtocolID,
		0,
		(&rem.ReserveUsernameArg{N: username}).Export(),
		1,
	)
	if err != nil {
		return err
	}
	signupFrame, err := rpcRequestFrameAt(rem.RegProtocolID, 2, argument.Export(), 1)
	if err != nil {
		return err
	}
	pukEntity, err := puk.EntityID()
	if err != nil {
		return err
	}
	uidEntity, err := pukEntity.Persistent(proto.PartyType_User)
	if err != nil {
		return err
	}
	uid, err := uidEntity.ToUID()
	if err != nil {
		return err
	}
	if !opened.Uid.Eq(uid) {
		return core.LinkError("opened eldest UID mismatch")
	}
	w := writer{dir: output}
	objects := []struct {
		name string
		obj  core.Encodeable
	}{
		{"reservation.snowp", &reservation},
		{"eldest-link.snowp", eldest.Link},
		{"puk-box-set.snowp", pukBox},
		{"hepk-set.snowp", eldest.HEPKSet},
		{"uid.snowp", &uid},
	}
	for _, object := range objects {
		if err := w.object(object.name, object.obj); err != nil {
			return err
		}
	}
	raw := []struct {
		name string
		data []byte
	}{
		{"device-seed.bin", deviceSeed[:]},
		{"puk-seed.bin", pukSeed[:]},
		{"next-tree-location.bin", eldest.NextTreeLocation[:]},
		{"subchain-tree-location.bin", eldest.SubchainTreeLocationSeed[:]},
		{"username-commitment-key.bin", eldest.UsernameCommitmentKey[:]},
		{"device-commitment-key.bin", eldest.DevNameCommitmentKey[:]},
		{"self-token.bin", selfToken[:]},
		{"reserve-request.frame", reserveFrame},
		{"signup-request.frame", signupFrame},
	}
	for _, file := range raw {
		if err := w.raw(file.name, file.data); err != nil {
			return err
		}
	}
	sort.Slice(w.files, func(i, j int) bool { return w.files[i].File < w.files[j].File })
	sort.Slice(w.rpcFiles, func(i, j int) bool { return w.rpcFiles[i].File < w.rpcFiles[j].File })
	manifest := struct {
		Format            string    `json:"format"`
		FOKSVersion       string    `json:"foks_version"`
		GeneratedAt       string    `json:"generated_at"`
		GoModuleGenerated bool      `json:"go_module_generated"`
		Files             []fixture `json:"files"`
		RawFiles          []fixture `json:"raw_files"`
	}{
		Format:            "foks-v0.1.9-signup-fixtures-v2",
		FOKSVersion:       "v0.1.9",
		GeneratedAt:       time.Now().UTC().Format(time.RFC3339),
		GoModuleGenerated: true,
		Files:             w.files,
		RawFiles:          w.rpcFiles,
	}
	encoded, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	encoded = append(encoded, '\n')
	return os.WriteFile(filepath.Join(output, "manifest.json"), encoded, 0o644)
}
