package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"time"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/merkle"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
	"github.com/keybase/go-codec/codec"
)

type fixtureMerkleEntry struct {
	key   proto.MerkleTreeRFOutput
	value proto.StdHash
}

type fixtureMerkleNode struct {
	leaf     *fixtureMerkleEntry
	interior *proto.MerkleInteriorNode
	left     *fixtureMerkleNode
	right    *fixtureMerkleNode
	hash     proto.MerkleNodeHash
}

func fixtureBitAt(key proto.MerkleTreeRFOutput, bit int) bool {
	return (key[bit>>3] & (1 << (7 - (bit & 7)))) != 0
}

func fixtureCommonPrefix(entries []fixtureMerkleEntry, start int) int {
	count := 0
	for bit := start; bit < len(entries[0].key)*8; bit++ {
		value := fixtureBitAt(entries[0].key, bit)
		for _, entry := range entries[1:] {
			if fixtureBitAt(entry.key, bit) != value {
				return count
			}
		}
		count++
	}
	return count
}

func buildFixtureMerkleTree(entries []fixtureMerkleEntry, start int) (*fixtureMerkleNode, error) {
	if len(entries) == 1 {
		leaf := proto.NewMerkleNodeWithLeaf(proto.MerkleLeaf{Key: entries[0].key, Value: entries[0].value})
		ret := &fixtureMerkleNode{leaf: &entries[0]}
		if err := merkle.HashNode(&leaf, &ret.hash); err != nil {
			return nil, err
		}
		return ret, nil
	}
	prefixCount := fixtureCommonPrefix(entries, start)
	branch := start + prefixCount
	split := sort.Search(len(entries), func(i int) bool { return fixtureBitAt(entries[i].key, branch) })
	if split == 0 || split == len(entries) {
		return nil, fmt.Errorf("failed to split fixture Merkle entries")
	}
	left, err := buildFixtureMerkleTree(entries[:split], branch+1)
	if err != nil {
		return nil, err
	}
	right, err := buildFixtureMerkleTree(entries[split:], branch+1)
	if err != nil {
		return nil, err
	}
	prefix := core.CopyAndClamp(entries[0].key[:], start, prefixCount)
	interior := proto.MerkleInteriorNode{
		PrefixBitStart: uint64(start),
		PrefixBitCount: uint64(prefixCount),
		Prefix:         prefix,
		Left:           left.hash,
		Right:          right.hash,
	}
	wire := proto.NewMerkleNodeWithNode(interior)
	ret := &fixtureMerkleNode{
		interior: &interior,
		left:     left,
		right:    right,
	}
	if err := merkle.HashNode(&wire, &ret.hash); err != nil {
		return nil, err
	}
	return ret, nil
}

func fixtureMerklePath(root *fixtureMerkleNode, key proto.MerkleTreeRFOutput) (proto.MerklePathCompressedPair, error) {
	var path proto.MerklePathCompressedBlob
	cursor := root
	bit := 0
	for cursor.leaf == nil {
		count := int(cursor.interior.PrefixBitCount)
		if err := core.AssertKeyMatch(key[:], cursor.interior.Prefix, bit, count); err != nil {
			return proto.MerklePathCompressedPair{
				Path: path,
				Terminal: proto.NewMerklePathTerminalWithFalse(proto.MerklePathIncomplete{
					NodeAtPrefixMiss: *cursor.interior,
				}),
			}, nil
		}
		branch := fixtureBitAt(key, bit+count)
		path = append(path, byte(count))
		if branch {
			path = append(path, cursor.left.hash[:]...)
			cursor = cursor.right
		} else {
			path = append(path, cursor.right.hash[:]...)
			cursor = cursor.left
		}
		bit += count + 1
	}
	terminal := proto.MerklePathToLeaf{Leaf: cursor.leaf.value}
	if cursor.leaf.key != key {
		found := cursor.leaf.key
		terminal.FoundKey = &found
	}
	return proto.MerklePathCompressedPair{
		Path:     path,
		Terminal: proto.NewMerklePathTerminalWithTrue(terminal),
	}, nil
}

func fixtureTree(entries []fixtureMerkleEntry, queries []proto.MerkleTreeRFOutput) (proto.MerkleNodeHash, []proto.MerklePathCompressedPair, error) {
	sort.Slice(entries, func(i, j int) bool { return bytes.Compare(entries[i].key[:], entries[j].key[:]) < 0 })
	root, err := buildFixtureMerkleTree(entries, 0)
	if err != nil {
		return proto.MerkleNodeHash{}, nil, err
	}
	paths := make([]proto.MerklePathCompressedPair, len(queries))
	for i, query := range queries {
		paths[i], err = fixtureMerklePath(root, query)
		if err != nil {
			return proto.MerkleNodeHash{}, nil, err
		}
	}
	return root.hash, paths, nil
}

func fixtureMerkleRoot(
	epoch proto.MerkleEpno,
	rootNode proto.MerkleNodeHash,
	hostchain proto.HostchainTail,
	hashes map[proto.MerkleEpno]proto.MerkleRootHash,
) (proto.MerkleRoot, proto.MerkleRootHash, proto.MerkleBackPointers, error) {
	sequence := merkle.MerkleBackpointerSequence(epoch)
	pointers := make(proto.MerkleBackPointers, len(sequence))
	for i, target := range sequence {
		hash, ok := hashes[target]
		if !ok {
			return proto.MerkleRoot{}, proto.MerkleRootHash{}, nil, fmt.Errorf("missing fixture root hash at epoch %d", target)
		}
		pointers[i] = proto.MerkleBackPointer{Epno: target, Hash: hash}
	}
	var pointerHash proto.MerkleBackPointerHash
	if err := merkle.HashBackPointers(&pointers, &pointerHash); err != nil {
		return proto.MerkleRoot{}, proto.MerkleRootHash{}, nil, err
	}
	root := proto.NewMerkleRootWithV1(proto.MerkleRootV1{
		Epno:         epoch,
		Time:         proto.Now(),
		BackPointers: pointerHash,
		RootNode:     rootNode,
		Hostchain:    hostchain,
	})
	var hash proto.MerkleRootHash
	if err := merkle.HashRoot(&root, &hash); err != nil {
		return proto.MerkleRoot{}, proto.MerkleRootHash{}, nil, err
	}
	hashes[epoch] = hash
	return root, hash, pointers, nil
}

type userFixtureManifest struct {
	Format          string    `json:"format"`
	FOKSVersion     string    `json:"foks_version"`
	GeneratedAt     string    `json:"generated_at"`
	HostID          string    `json:"host_id"`
	UID             string    `json:"uid"`
	DeviceID        string    `json:"device_id"`
	UserRootHash    string    `json:"user_root_hash"`
	UserMerkleKey   string    `json:"user_merkle_key"`
	UserLinkHash    string    `json:"user_link_hash"`
	PUKGeneration   uint64    `json:"puk_generation"`
	OfficialUnboxed bool      `json:"official_unboxed"`
	Files           []fixture `json:"files"`
	RawFiles        []fixture `json:"raw_files"`
}

func rpcRequestFrame[D any](protocol rpc.ProtocolUniqueID, position rpc.Position, data D) ([]byte, error) {
	warg := &rpc.DataWrap[proto.Header, D]{
		Header: core.MakeProtoHeader(),
		Data:   data,
	}
	frame := []interface{}{
		rpc.MethodCallV2,
		rpc.SeqNumber(0),
		protocol,
		position,
		warg,
	}
	handle := core.Codec()
	var content []byte
	if err := codec.NewEncoderBytes(&content, handle).Encode(frame); err != nil {
		return nil, err
	}
	var length []byte
	if err := codec.NewEncoderBytes(&length, handle).Encode(len(content)); err != nil {
		return nil, err
	}
	return append(length, content...), nil
}

func writeUserFixtures(output string, address proto.TCPAddr, hostID proto.HostID, trustedRoot *proto.MerkleRoot) error {
	if err := os.MkdirAll(output, 0o755); err != nil {
		return err
	}
	owner := proto.NewRoleDefault(proto.RoleType_OWNER)
	var deviceSeed proto.SecretSeed32
	var pukSeed proto.SecretSeed32
	for i := range deviceSeed {
		deviceSeed[i] = byte(i + 1)
		pukSeed[i] = byte(i + 101)
	}
	device, err := core.NewPrivateSuite25519(proto.EntityType_Device, owner, deviceSeed, hostID)
	if err != nil {
		return err
	}
	puk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_User,
		owner,
		pukSeed,
		proto.FirstGeneration,
		hostID,
	)
	if err != nil {
		return err
	}
	trustedTreeRoot, err := merkle.ToTreeRoot(trustedRoot)
	if err != nil {
		return err
	}
	username := proto.Name("fixtureuser").Normalize()
	deviceLabel := proto.DeviceLabel{
		DeviceType: proto.DeviceType_Computer,
		Name:       proto.DeviceNameNormalized("fixture-device"),
		Serial:     proto.FirstDeviceSerial,
	}
	deviceLabelAndName := proto.DeviceLabelAndName{
		Label: deviceLabel,
		Nv:    proto.NormalizationVersion_V0,
		Name:  proto.DeviceName("fixture-device"),
	}
	eldest, err := core.MakeEldestLink(
		hostID,
		rem.NameCommitment{Name: username, Seq: proto.FirstNameSeqno},
		device,
		puk,
		deviceLabel,
		*trustedTreeRoot,
		nil,
	)
	if err != nil {
		return err
	}
	hepks, err := core.ImportHEPKSet(eldest.HEPKSet)
	if err != nil {
		return err
	}
	opened, err := core.OpenEldestLink(eldest.Link, hepks, hostID)
	if err != nil {
		return err
	}
	uid := opened.Uid
	deviceID, err := device.DeviceID()
	if err != nil {
		return err
	}
	devicePublic, err := device.Publicize(&hostID)
	if err != nil {
		return err
	}
	linkHash, err := core.LinkHash(eldest.Link)
	if err != nil {
		return err
	}
	firstInput := proto.MerkleTreeRFInput{
		Ct:     proto.ChainType_User,
		Entity: uid.EntityID(),
		Seqno:  proto.ChainEldestSeqno,
	}
	var firstKey proto.MerkleTreeRFOutput
	if err := merkle.KeyHash(&firstKey, firstInput); err != nil {
		return err
	}
	nameEntityID, err := merkle.NameToEntityID(username, hostID)
	if err != nil {
		return err
	}
	firstNameKey, err := merkle.HashName(nameEntityID, proto.FirstNameSeqno)
	if err != nil {
		return err
	}
	nextNameKey, err := merkle.HashName(nameEntityID, proto.FirstNameSeqno+1)
	if err != nil {
		return err
	}
	nameValue := rem.NewEntityIDMerkleValueWithV1(uid.EntityID())
	nameLeaf, err := core.PrefixedHash(&nameValue)
	if err != nil {
		return err
	}
	trustedV1 := trustedRoot.V1()
	var trustedHash proto.MerkleRootHash
	if err := merkle.HashRoot(trustedRoot, &trustedHash); err != nil {
		return err
	}
	hashes := map[proto.MerkleEpno]proto.MerkleRootHash{trustedV1.Epno: trustedHash}
	for _, epoch := range merkle.MerkleBackpointerSequence(trustedV1.Epno + 1) {
		if _, found := hashes[epoch]; !found {
			sum := sha256.Sum256([]byte(fmt.Sprintf("foks-v0.1.9-fixture-root-%d", epoch)))
			hashes[epoch] = proto.MerkleRootHash(sum)
		}
	}
	entries := []fixtureMerkleEntry{
		{key: *firstNameKey, value: *nameLeaf},
		{key: firstKey, value: linkHash.ToStdHash()},
	}
	rootNode996, _, err := fixtureTree(append([]fixtureMerkleEntry(nil), entries...), []proto.MerkleTreeRFOutput{firstKey})
	if err != nil {
		return err
	}
	root996, hash996, backPointers996, err := fixtureMerkleRoot(
		trustedV1.Epno+1, rootNode996, trustedV1.Hostchain, hashes,
	)
	if err != nil {
		return err
	}
	treeRoot996 := proto.TreeRoot{Epno: trustedV1.Epno + 1, Hash: hash996}

	var secondDeviceSeed proto.SecretSeed32
	for i := range secondDeviceSeed {
		secondDeviceSeed[i] = byte(i + 33)
	}
	secondDevice, err := core.NewPrivateSuite25519(proto.EntityType_Device, owner, secondDeviceSeed, hostID)
	if err != nil {
		return err
	}
	secondLabel := proto.DeviceLabel{
		DeviceType: proto.DeviceType_Computer,
		Name:       proto.DeviceNameNormalized("fixture-device-two"),
		Serial:     proto.FirstDeviceSerial + 1,
	}
	provision, err := core.MakeProvisionLink(
		uid, hostID, devicePublic, secondDevice, owner, nil, secondLabel,
		proto.ChainEldestSeqno+1, *linkHash, treeRoot996, nil,
	)
	if err != nil {
		return err
	}
	provision.Link, err = core.CountersignProvisionLink(provision.Link, device)
	if err != nil {
		return err
	}
	provisionHash, err := core.LinkHash(provision.Link)
	if err != nil {
		return err
	}
	secondInput := firstInput
	secondInput.Seqno++
	secondInput.Location = eldest.NextTreeLocation
	var secondKey proto.MerkleTreeRFOutput
	if err := merkle.KeyHash(&secondKey, secondInput); err != nil {
		return err
	}
	entries = append(entries, fixtureMerkleEntry{key: secondKey, value: provisionHash.ToStdHash()})
	rootNode997, _, err := fixtureTree(append([]fixtureMerkleEntry(nil), entries...), []proto.MerkleTreeRFOutput{firstKey, secondKey})
	if err != nil {
		return err
	}
	root997, hash997, backPointers997, err := fixtureMerkleRoot(
		trustedV1.Epno+2, rootNode997, trustedV1.Hostchain, hashes,
	)
	if err != nil {
		return err
	}
	treeRoot997 := proto.TreeRoot{Epno: trustedV1.Epno + 2, Hash: hash997}

	var rotatedPukSeed proto.SecretSeed32
	for i := range rotatedPukSeed {
		rotatedPukSeed[i] = byte(i + 151)
	}
	rotatedPuk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_User, owner, rotatedPukSeed, proto.FirstGeneration+1, hostID,
	)
	if err != nil {
		return err
	}
	secondPublic, err := secondDevice.Publicize(&hostID)
	if err != nil {
		return err
	}
	revoke, err := core.MakeRevokeLink(
		uid, hostID, device, secondPublic, []core.SharedPrivateSuiter{rotatedPuk},
		proto.ChainEldestSeqno+2, *provisionHash, treeRoot997,
	)
	if err != nil {
		return err
	}
	revokeHash, err := core.LinkHash(revoke.Link)
	if err != nil {
		return err
	}
	thirdInput := secondInput
	thirdInput.Seqno++
	thirdInput.Location = provision.NextTreeLocation
	var thirdKey proto.MerkleTreeRFOutput
	if err := merkle.KeyHash(&thirdKey, thirdInput); err != nil {
		return err
	}
	entries = append(entries, fixtureMerkleEntry{key: thirdKey, value: revokeHash.ToStdHash()})
	nextInput := thirdInput
	nextInput.Seqno++
	nextInput.Location = revoke.NextTreeLocation
	var nextKey proto.MerkleTreeRFOutput
	if err := merkle.KeyHash(&nextKey, nextInput); err != nil {
		return err
	}
	queries := []proto.MerkleTreeRFOutput{*firstNameKey, *nextNameKey, firstKey, secondKey, thirdKey, nextKey}
	rootNode998, compressedPaths, err := fixtureTree(append([]fixtureMerkleEntry(nil), entries...), queries)
	if err != nil {
		return err
	}
	userRoot, rootHash, backPointers998, err := fixtureMerkleRoot(
		trustedV1.Epno+3, rootNode998, trustedV1.Hostchain, hashes,
	)
	if err != nil {
		return err
	}
	paths := proto.MerklePathsCompressed{Root: userRoot, Paths: compressedPaths}
	for index, key := range queries {
		path := paths.Select(index)
		present := index == 0 || (index >= 2 && index < 5)
		_, err := merkle.Verify(&path, &key, present)
		if err != nil {
			return fmt.Errorf("official Merkle verification %d: %w", index, err)
		}
	}

	allHEPKs := proto.HEPKSet{V: append(append(append([]proto.HEPK{}, eldest.HEPKSet.V...), provision.HEPKSet.V...), revoke.HEPKSet.V...)}
	allHEPKSet, err := core.ImportHEPKSet(&allHEPKs)
	if err != nil {
		return err
	}
	if _, err := core.OpenDeviceChange(provision.Link, allHEPKSet, &uid, hostID); err != nil {
		return fmt.Errorf("official provision-link verification: %w", err)
	}
	if _, err := core.OpenDeviceChange(revoke.Link, allHEPKSet, &uid, hostID); err != nil {
		return fmt.Errorf("official revoke-link verification: %w", err)
	}

	boxSet, err := core.BoxOne(hostID, rotatedPuk, device, devicePublic)
	if err != nil {
		return err
	}
	if len(boxSet.Boxes) != 1 {
		return fmt.Errorf("expected one PUK box, got %d", len(boxSet.Boxes))
	}
	parcel := proto.SharedKeyParcel{Box: boxSet.Boxes[0], Sender: deviceID.EntityID(), BoxId: boxSet.Id, TempDHKeySigned: boxSet.TempDHKeySigned}
	var unboxed proto.SharedKeySeed
	if err := core.OpenBoxInSet(&unboxed, parcel.Box.Box, parcel.TempDHKeySigned, &parcel.BoxId, devicePublic, device); err != nil {
		return err
	}
	if !bytes.Equal(unboxed.Seed[:], rotatedPukSeed[:]) {
		return fmt.Errorf("official PUK unbox returned the wrong rotated seed")
	}
	boxed, err := core.OpenBoxHybrid(parcel.Box.Box)
	if err != nil {
		return err
	}
	dhSecret, err := core.DeviceDHSecretKey(deviceSeed)
	if err != nil {
		return err
	}
	senderDH, err := devicePublic.Curve25519()
	if err != nil {
		return err
	}
	dhShared, err := core.Curve25519DHExchange(dhSecret, senderDH)
	if err != nil {
		return err
	}
	kemSeed, err := core.PQKemAlgo.DeriveSeed(deviceSeed)
	if err != nil {
		return err
	}
	_, kemSecret, err := core.PQKemAlgo.KeyFromSeed(kemSeed)
	if err != nil {
		return err
	}
	kemShared, err := core.PQKemAlgo.Decap(kemSecret, boxed.KemCtext)
	if err != nil {
		return err
	}
	receiverHEPK, err := device.ExportHEPK()
	if err != nil {
		return err
	}
	senderDHPublic, err := devicePublic.DHPublicKey()
	if err != nil {
		return err
	}
	hybridPayload := proto.HybridSecretKeySHA3Payload{Version: proto.BoxHybridVersion_V1, PqKemKey: kemShared, DhSharedKey: dhShared.ToDHSharedKey(), Rcvr: *receiverHEPK, Sndr: *senderDHPublic}
	var hybridKey proto.SecretBoxKey
	if err := core.PrefixedSHA3HashInto(&hybridPayload, hybridKey[:]); err != nil {
		return err
	}

	userChain := rem.UserChain{
		Links:     []proto.LinkOuter{*eldest.Link, *provision.Link, *revoke.Link},
		Locations: []proto.TreeLocation{*eldest.NextTreeLocation, *provision.NextTreeLocation, *revoke.NextTreeLocation},
		Usernames: []rem.NameCommitmentAndKey{
			{Unc: rem.NameCommitment{Name: username, Seq: proto.FirstNameSeqno}, Key: *eldest.UsernameCommitmentKey},
		},
		Merkle: paths,
		DeviceNames: []rem.DeviceLabelNameAndCommitmentKey{
			{Dln: deviceLabelAndName, CommitmentKey: *eldest.DevNameCommitmentKey},
			{Dln: proto.DeviceLabelAndName{Label: secondLabel, Nv: proto.NormalizationVersion_V0, Name: proto.DeviceName("fixture-device-two")}, CommitmentKey: *provision.DevNameCommitmentKey},
		},
		UsernameUtf8:     proto.NameUtf8("fixtureuser"),
		NumUsernameLinks: 2,
		Hepks:            allHEPKs,
	}

	loadArg := rem.UserLoadUserChainArg{A: rem.LoadUserChainArg{
		Uid:   uid,
		Start: proto.ChainEldestSeqno,
		Auth:  rem.NewLoadUserChainAuthWithAslocaluser(),
	}}
	loadFrame, err := rpcRequestFrame(rem.UserProtocolID, 9, loadArg.Export())
	if err != nil {
		return err
	}
	pukArg := rem.GetPUKForRoleArg{Role: owner, TargetPublicKeyId: deviceID.EntityID()}
	pukFrame, err := rpcRequestFrame(rem.UserProtocolID, 14, pukArg.Export())
	if err != nil {
		return err
	}
	certArg := rem.GetClientCertChainArg{Uid: uid, Key: deviceID.EntityID()}
	certFrame, err := rpcRequestFrame(rem.RegProtocolID, 1, certArg.Export())
	if err != nil {
		return err
	}
	currentRootFrame, err := rpcRequestFrame(rem.MerkleQueryProtocolID, 2, (*proto.HostID)(nil))
	if err != nil {
		return err
	}
	historicalFull, historicalHashEpochs := merkle.MerkleCollectRoots(trustedV1.Epno+3, trustedV1.Epno)
	historicalFull = historicalFull[1:]
	historicalHashEpochs = slices.DeleteFunc(historicalHashEpochs, func(epoch proto.MerkleEpno) bool {
		return epoch == trustedV1.Epno
	})
	historicalArg := rem.GetHistoricalRootsArg{Full: historicalFull, Hashes: historicalHashEpochs}
	historicalFrame, err := rpcRequestFrame(rem.MerkleQueryProtocolID, 1, historicalArg.Export())
	if err != nil {
		return err
	}
	historicalHashes := make([]proto.MerkleRootHash, len(historicalHashEpochs))
	for i, epoch := range historicalHashEpochs {
		historicalHashes[i] = hashes[epoch]
	}
	historicalRes := rem.GetHistoricalRootsRes{Roots: []proto.MerkleRoot{root996}, Hashes: historicalHashes}

	w := writer{dir: output}
	objects := []struct {
		name string
		obj  core.Encodeable
	}{
		{"device-id.snowp", &deviceID},
		{"uid.snowp", &uid},
		{"hepk-set.snowp", &allHEPKs},
		{"user-eldest-link.snowp", eldest.Link},
		{"user-provision-link.snowp", provision.Link},
		{"user-revoke-link.snowp", revoke.Link},
		{"user-merkle-paths.snowp", &paths},
		{"user-chain.snowp", &userChain},
		{"merkle-root-996.snowp", &root996},
		{"merkle-root-997.snowp", &root997},
		{"merkle-root-998.snowp", &userRoot},
		{"merkle-back-pointers-996.snowp", &backPointers996},
		{"merkle-back-pointers-997.snowp", &backPointers997},
		{"merkle-back-pointers-998.snowp", &backPointers998},
		{"merkle-historical-response.snowp", &historicalRes},
		{"puk-parcel.snowp", &parcel},
		{"puk-cleartext.snowp", &unboxed},
		{"hybrid-payload.snowp", &hybridPayload},
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
		{"second-device-seed.bin", secondDeviceSeed[:]},
		{"initial-puk-seed.bin", pukSeed[:]},
		{"puk-seed.bin", rotatedPukSeed[:]},
		{"hybrid-dh-shared.bin", dhShared[:]},
		{"hybrid-kem-shared.bin", kemShared[:]},
		{"hybrid-secretbox-key.bin", hybridKey[:]},
		{"reg-cert-request.frame", certFrame},
		{"user-load-request.frame", loadFrame},
		{"user-puk-request.frame", pukFrame},
		{"merkle-current-root-request.frame", currentRootFrame},
		{"merkle-historical-roots-request.frame", historicalFrame},
	}
	for _, file := range raw {
		if err := w.raw(file.name, file.data); err != nil {
			return err
		}
	}
	sort.Slice(w.files, func(i, j int) bool { return w.files[i].File < w.files[j].File })
	sort.Slice(w.rpcFiles, func(i, j int) bool { return w.rpcFiles[i].File < w.rpcFiles[j].File })
	uidString, err := uid.StringErr()
	if err != nil {
		return err
	}
	deviceIDString, err := deviceID.StringErr()
	if err != nil {
		return err
	}
	manifest := userFixtureManifest{
		Format:          "foks-v0.1.9-user-fixtures-v1",
		FOKSVersion:     "v0.1.9",
		GeneratedAt:     time.Now().UTC().Format(time.RFC3339),
		HostID:          hostID.String(),
		UID:             uidString,
		DeviceID:        deviceIDString,
		UserRootHash:    hex.EncodeToString(rootHash[:]),
		UserMerkleKey:   hex.EncodeToString(firstKey[:]),
		UserLinkHash:    revokeHash.String(),
		PUKGeneration:   uint64(proto.FirstGeneration + 1),
		OfficialUnboxed: true,
		Files:           w.files,
		RawFiles:        w.rpcFiles,
	}
	encoded, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	encoded = append(encoded, '\n')
	if err := os.WriteFile(filepath.Join(output, "manifest.json"), encoded, 0o644); err != nil {
		return err
	}
	for _, file := range append(append([]fixture{}, w.files...), w.rpcFiles...) {
		data, err := os.ReadFile(filepath.Join(output, file.File))
		if err != nil {
			return err
		}
		hash := sha256.Sum256(data)
		if hex.EncodeToString(hash[:]) != file.SHA256 {
			return fmt.Errorf("fixture digest changed while writing %s", file.File)
		}
	}
	fmt.Printf("generated user %s with device %s for %s\n", uidString, deviceIDString, address)
	return nil
}
