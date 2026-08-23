package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/foks-proj/go-foks/client/libyubi"
	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
)

// TestGenerateYubiFixtures is deliberately opt-in: it uses FOKS's software
// YubiKey bus, never a physical token, and writes fixtures only when invoked
// from the documented command line.
func TestGenerateYubiFixtures(t *testing.T) {
	output := os.Getenv("FOKS_YUBI_FIXTURE_OUT")
	if output == "" {
		t.Skip("set FOKS_YUBI_FIXTURE_OUT to generate the checked-in matrix")
	}
	if err := os.MkdirAll(output, 0o755); err != nil {
		t.Fatal(err)
	}

	host := core.RandomHostID()
	role := proto.OwnerRole
	dispatch, err := libyubi.AllocDispatchTest()
	if err != nil {
		t.Fatal(err)
	}
	yubi := dispatch.NextTestKey(context.Background(), t, role, host)
	yubiPublic, err := yubi.Publicize(&host)
	if err != nil {
		t.Fatal(err)
	}
	yubiHEPK, err := yubi.ExportHEPK()
	if err != nil {
		t.Fatal(err)
	}

	subkey, subkeyBox, err := core.MakeSubkey(yubi, host)
	if err != nil {
		t.Fatal(err)
	}
	subkeyPublic, err := subkey.EntityPublic()
	if err != nil {
		t.Fatal(err)
	}
	subkeyID := subkeyPublic.GetEntityID()

	var pukSeed proto.SecretSeed32
	var softwareSeed proto.SecretSeed32
	for i := range pukSeed {
		pukSeed[i] = byte(i + 81)
		softwareSeed[i] = byte(i + 17)
	}
	puk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_User, role, pukSeed, proto.FirstGeneration, host,
	)
	if err != nil {
		t.Fatal(err)
	}
	software, err := core.NewPrivateSuite25519(
		proto.EntityType_Device, role, softwareSeed, host,
	)
	if err != nil {
		t.Fatal(err)
	}
	softwarePublic, err := software.Publicize(&host)
	if err != nil {
		t.Fatal(err)
	}

	label := proto.DeviceLabel{
		DeviceType: proto.DeviceType_YubiKey,
		Name:       proto.DeviceNameNormalized("fixture-yubikey"),
		Serial:     proto.FirstDeviceSerial,
	}
	eldest, err := core.MakeEldestLink(
		host,
		rem.NameCommitment{Name: proto.Name("fixtureyubi"), Seq: proto.FirstNameSeqno},
		yubi,
		puk,
		label,
		proto.TreeRoot{Epno: 1, Hash: proto.MerkleRootHash{}},
		subkey,
	)
	if err != nil {
		t.Fatal(err)
	}
	hepks, err := core.ImportHEPKSet(eldest.HEPKSet)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := core.OpenEldestLink(eldest.Link, hepks, host); err != nil {
		t.Fatalf("official Yubi/subkey eldest verification: %v", err)
	}

	toYubi, err := core.BoxOne(host, puk, software, yubiPublic)
	if err != nil {
		t.Fatal(err)
	}
	toSoftware, err := core.BoxOne(host, puk, yubi, softwarePublic)
	if err != nil {
		t.Fatal(err)
	}
	toYubiParcel := proto.SharedKeyParcel{
		Box:             toYubi.Boxes[0],
		Sender:          mustEntityID(t, software),
		BoxId:           toYubi.Id,
		TempDHKeySigned: toYubi.TempDHKeySigned,
	}
	toSoftwareParcel := proto.SharedKeyParcel{
		Box:             toSoftware.Boxes[0],
		Sender:          mustEntityID(t, yubi),
		BoxId:           toSoftware.Id,
		TempDHKeySigned: toSoftware.TempDHKeySigned,
	}
	var clear proto.SharedKeySeed
	if err := core.OpenBoxInSet(
		&clear, toYubiParcel.Box.Box, toYubiParcel.TempDHKeySigned,
		&toYubiParcel.BoxId, softwarePublic, yubi,
	); err != nil {
		t.Fatalf("official software-to-Yubi unbox: %v", err)
	}
	if err := core.OpenBoxInSet(
		&clear, toSoftwareParcel.Box.Box, toSoftwareParcel.TempDHKeySigned,
		&toSoftwareParcel.BoxId, yubiPublic, software,
	); err != nil {
		t.Fatalf("official Yubi-to-software unbox: %v", err)
	}

	w := writer{dir: output}
	yubiID := mustEntityID(t, yubi)
	objects := []struct {
		name string
		obj  core.Encodeable
	}{
		{"yubi-id.snowp", &yubiID},
		{"yubi-hepk.snowp", yubiHEPK},
		{"subkey-id.snowp", &subkeyID},
		{"subkey-box.snowp", subkeyBox},
		{"yubi-eldest-link.snowp", eldest.Link},
		{"software-to-yubi-puk-parcel.snowp", &toYubiParcel},
		{"yubi-to-software-puk-parcel.snowp", &toSoftwareParcel},
	}
	for _, object := range objects {
		if err := w.object(object.name, object.obj); err != nil {
			t.Fatal(err)
		}
	}
	if err := w.raw("software-device-seed.bin", softwareSeed[:]); err != nil {
		t.Fatal(err)
	}
	if err := w.raw("puk-seed.bin", pukSeed[:]); err != nil {
		t.Fatal(err)
	}
	sort.Slice(w.files, func(i, j int) bool { return w.files[i].File < w.files[j].File })
	sort.Slice(w.rpcFiles, func(i, j int) bool { return w.rpcFiles[i].File < w.rpcFiles[j].File })
	manifest := struct {
		Format      string    `json:"format"`
		FOKSVersion string    `json:"foks_version"`
		Files       []fixture `json:"files"`
		RawFiles    []fixture `json:"raw_files"`
	}{
		Format:      "foks-v0.1.9-yubi-subkey-fixtures-v1",
		FOKSVersion: "v0.1.9",
		Files:       w.files,
		RawFiles:    w.rpcFiles,
	}
	encoded, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	encoded = append(encoded, '\n')
	if err := os.WriteFile(filepath.Join(output, "manifest.json"), encoded, 0o644); err != nil {
		t.Fatal(err)
	}
}

func mustEntityID(t *testing.T, suite core.PrivateSuiter) proto.EntityID {
	t.Helper()
	id, err := suite.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	return id
}
