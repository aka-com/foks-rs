package main

import (
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/merkle"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
)

// writeBackupFixtures uses only the official v0.1.9 key, chain, boxing, and
// generated RPC types. It covers both enrolling a backup key and using that
// ephemeral key to provision a replacement software device.
func writeBackupFixtures(w *writer, userDir string, chain *rem.UserChain) error {
	var uid proto.UID
	if err := decodeFixture(filepath.Join(userDir, "uid.snowp"), &uid); err != nil {
		return err
	}
	last := chain.Links[len(chain.Links)-1]
	change, _, err := core.OpenGroupChange(&last)
	if err != nil {
		return err
	}
	host := change.Entity.Host
	previous, err := core.LinkHash(&last)
	if err != nil {
		return err
	}
	root, err := merkle.ToTreeRoot(&chain.Merkle.Root)
	if err != nil {
		return err
	}

	deviceSeedBytes, err := os.ReadFile(filepath.Join(userDir, "device-seed.bin"))
	if err != nil {
		return err
	}
	var deviceSeed proto.SecretSeed32
	copy(deviceSeed[:], deviceSeedBytes)
	device, err := core.NewPrivateSuite25519(proto.EntityType_Device, proto.OwnerRole, deviceSeed, host)
	if err != nil {
		return err
	}
	devicePublic, err := device.Publicize(&host)
	if err != nil {
		return err
	}

	pukSeedBytes, err := os.ReadFile(filepath.Join(userDir, "puk-seed.bin"))
	if err != nil {
		return err
	}
	var pukSeed proto.SecretSeed32
	copy(pukSeed[:], pukSeedBytes)
	puk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_User, proto.OwnerRole, pukSeed, proto.FirstGeneration+1, host,
	)
	if err != nil {
		return err
	}

	var raw proto.BackupSeed
	for i := range raw {
		raw[i] = byte(i + 1)
	}
	raw[0] &= 0x07
	var backup core.BackupKey
	if err := backup.FromSeed(raw); err != nil {
		return err
	}
	phrase, err := backup.Export()
	if err != nil {
		return err
	}
	backupKey, err := backup.KeySuite(proto.OwnerRole, host)
	if err != nil {
		return err
	}
	backupPublic, err := backupKey.Publicize(&host)
	if err != nil {
		return err
	}
	backupID, err := backupKey.EntityID()
	if err != nil {
		return err
	}
	backupHEPK, err := backupKey.ExportHEPK()
	if err != nil {
		return err
	}
	var derived proto.SecretSeed32
	if err := backup.SecretSeed32(&derived); err != nil {
		return err
	}
	labelAndName, err := backup.DeviceLabelAndName()
	if err != nil {
		return err
	}

	enroll, err := core.MakeProvisionLink(
		uid, host, devicePublic, backupKey, proto.OwnerRole, nil, labelAndName.Label,
		change.Chainer.Base.Seqno+1, *previous, *root, nil,
	)
	if err != nil {
		return err
	}
	enroll.Link, err = core.CountersignProvisionLink(enroll.Link, device)
	if err != nil {
		return err
	}
	enroll.Link, err = retimeUserGroupLink(enroll.Link, []core.Signer{backupKey, device})
	if err != nil {
		return err
	}
	enrollBoxes, err := core.BoxOne(host, puk, device, backupPublic)
	if err != nil {
		return err
	}
	var selfToken proto.PermissionToken
	for i := range selfToken {
		selfToken[i] = byte(31 + i)
	}
	enrollArg := rem.ProvisionDeviceArg{
		Link:             *enroll.Link,
		PukBoxes:         *enrollBoxes,
		Dlnc:             rem.DeviceLabelNameAndCommitmentKey{Dln: *labelAndName, CommitmentKey: *enroll.DevNameCommitmentKey},
		NextTreeLocation: *enroll.NextTreeLocation,
		SelfToken:        selfToken,
		Hepks:            *enroll.HEPKSet,
	}
	enrollFrame, err := rpcRequestFrame(rem.UserProtocolID, 6, enrollArg.Export())
	if err != nil {
		return err
	}

	enrollHash, err := core.LinkHash(enroll.Link)
	if err != nil {
		return err
	}
	var replacementSeed proto.SecretSeed32
	for i := range replacementSeed {
		replacementSeed[i] = byte(211 + i)
	}
	replacement, err := core.NewPrivateSuite25519(
		proto.EntityType_Device, proto.OwnerRole, replacementSeed, host,
	)
	if err != nil {
		return err
	}
	replacementLabel := proto.DeviceLabel{
		DeviceType: proto.DeviceType_Computer,
		Name:       proto.DeviceNameNormalized("recovered fixture device"),
		Serial:     proto.FirstDeviceSerial,
	}
	recoverLink, err := core.MakeProvisionLink(
		uid, host, backupPublic, replacement, proto.OwnerRole, nil, replacementLabel,
		change.Chainer.Base.Seqno+2, *enrollHash, *root, nil,
	)
	if err != nil {
		return err
	}
	recoverLink.Link, err = core.CountersignProvisionLink(recoverLink.Link, backupKey)
	if err != nil {
		return err
	}
	recoverLink.Link, err = retimeUserGroupLink(recoverLink.Link, []core.Signer{replacement, backupKey})
	if err != nil {
		return err
	}
	replacementPublic, err := replacement.Publicize(&host)
	if err != nil {
		return err
	}
	recoverBoxes, err := core.BoxOne(host, puk, backupKey, replacementPublic)
	if err != nil {
		return err
	}
	var recoverToken proto.PermissionToken
	for i := range recoverToken {
		recoverToken[i] = byte(71 + i)
	}
	recoverArg := rem.ProvisionDeviceArg{
		Link:     *recoverLink.Link,
		PukBoxes: *recoverBoxes,
		Dlnc: rem.DeviceLabelNameAndCommitmentKey{
			Dln: proto.DeviceLabelAndName{
				Label: replacementLabel,
				Nv:    proto.NormalizationVersion_V0,
				Name:  "Recovered Fixture Device",
			},
			CommitmentKey: *recoverLink.DevNameCommitmentKey,
		},
		NextTreeLocation: *recoverLink.NextTreeLocation,
		SelfToken:        recoverToken,
		Hepks:            *recoverLink.HEPKSet,
	}
	recoverFrame, err := rpcRequestFrame(rem.UserProtocolID, 6, recoverArg.Export())
	if err != nil {
		return err
	}

	var keyID proto.HMACKeyID
	var random proto.Random16
	var mac proto.HMAC
	for i := range keyID {
		keyID[i] = byte(101 + i)
	}
	for i := range random {
		random[i] = byte(121 + i)
	}
	for i := range mac {
		mac[i] = byte(141 + i)
	}
	challenge := rem.Challenge{
		Payload: rem.ChallengePayload{
			HmacKeyID: keyID,
			EntityID:  backupID,
			HostID:    host,
			Rand:      random,
			Time:      deterministicFixtureTime,
		},
		Mac: mac,
	}
	lookupSignature, err := backupKey.Sign(&challenge.Payload)
	if err != nil {
		return err
	}
	challengeArg := rem.GetUIDLookupChallegeArg{EntityID: backupID}
	challengeFrame, err := rpcRequestFrame(rem.RegProtocolID, 6, challengeArg.Export())
	if err != nil {
		return err
	}
	lookupArg := rem.LookupUIDByDeviceArg{EntityID: backupID, Challenge: challenge, Signature: *lookupSignature}
	lookupFrame, err := rpcRequestFrame(rem.RegProtocolID, 7, lookupArg.Export())
	if err != nil {
		return err
	}
	lookupResult := proto.LookupUserRes{
		Fqu:          proto.FQUser{Uid: uid, HostID: host},
		Username:     proto.Name("signupfixture"),
		UsernameUtf8: proto.NameUtf8("Signup Fixture"),
		Role:         proto.OwnerRole,
	}
	certArg := rem.GetClientCertChainArg{Uid: uid, Key: backupID}
	certFrame, err := rpcRequestFrameAt(rem.RegProtocolID, 1, certArg.Export(), 1)
	if err != nil {
		return err
	}

	for _, object := range []struct {
		name string
		obj  core.Encodeable
	}{
		{"backup-entity-id.snowp", &backupID},
		{"backup-hepk.snowp", backupHEPK},
		{"backup-enroll-link.snowp", enroll.Link},
		{"backup-enroll-boxes.snowp", enrollBoxes},
		{"backup-recover-link.snowp", recoverLink.Link},
		{"backup-recover-boxes.snowp", recoverBoxes},
		{"backup-lookup-challenge.snowp", &challenge},
		{"backup-lookup-signature.snowp", lookupSignature},
		{"backup-lookup-result.snowp", &lookupResult},
	} {
		if err := w.object(object.name, object.obj); err != nil {
			return err
		}
	}
	phraseJSON, err := json.Marshal(phrase)
	if err != nil {
		return err
	}
	for _, rawFixture := range []struct {
		name string
		data []byte
	}{
		{"backup-seed.bin", raw[:]},
		{"backup-derived-seed.bin", derived[:]},
		{"backup-phrase.json", phraseJSON},
		{"backup-enroll-request.frame", enrollFrame},
		{"backup-enroll-device-name-commitment-key.bin", enroll.DevNameCommitmentKey[:]},
		{"backup-enroll-next-tree-location.bin", enroll.NextTreeLocation[:]},
		{"backup-enroll-self-token.bin", selfToken[:]},
		{"backup-recover-device-seed.bin", replacementSeed[:]},
		{"backup-recover-request.frame", recoverFrame},
		{"backup-recover-device-name-commitment-key.bin", recoverLink.DevNameCommitmentKey[:]},
		{"backup-recover-next-tree-location.bin", recoverLink.NextTreeLocation[:]},
		{"backup-recover-self-token.bin", recoverToken[:]},
		{"backup-lookup-challenge-request.frame", challengeFrame},
		{"backup-lookup-request.frame", lookupFrame},
		{"backup-cert-request.frame", certFrame},
	} {
		if err := w.raw(rawFixture.name, rawFixture.data); err != nil {
			return err
		}
	}
	return nil
}
