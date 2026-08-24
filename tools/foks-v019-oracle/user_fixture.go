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
	"github.com/foks-proj/go-foks/lib/kv"
	"github.com/foks-proj/go-foks/lib/merkle"
	teamlib "github.com/foks-proj/go-foks/lib/team"
	"github.com/foks-proj/go-foks/proto/lcl"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
	"github.com/keybase/go-codec/codec"
	"golang.org/x/crypto/nacl/secretbox"
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
	return rpcRequestFrameAt(protocol, position, data, 0)
}

func rpcRequestFrameAt[D any](protocol rpc.ProtocolUniqueID, position rpc.Position, data D, sequence rpc.SeqNumber) ([]byte, error) {
	warg := &rpc.DataWrap[proto.Header, D]{
		Header: core.MakeProtoHeader(),
		Data:   data,
	}
	frame := []interface{}{
		rpc.MethodCallV2,
		sequence,
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

func rpcVoidResponseFrame(sequence rpc.SeqNumber) ([]byte, error) {
	wrapped := &rpc.DataWrap[proto.Header, interface{}]{Header: core.MakeProtoHeader()}
	frame := []interface{}{rpc.MethodResponse, sequence, (*proto.Status)(nil), wrapped}
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

func rpcErrorResponseFrame(sequence rpc.SeqNumber, status proto.Status) ([]byte, error) {
	wrapped := &rpc.DataWrap[proto.Header, interface{}]{Header: core.MakeProtoHeader()}
	frame := []interface{}{rpc.MethodResponse, sequence, &status, wrapped}
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
	historicalCleartext := puk.ExportToBoxCleartext(proto.FQEntity{Entity: uid.EntityID(), Host: hostID})
	historicalBoxKey := rotatedPuk.SecretBoxKey()
	historicalBox, err := core.SealIntoSecretBox(&historicalCleartext, &historicalBoxKey)
	if err != nil {
		return err
	}
	parcel.SeedChain = []proto.SeedChainBox{{
		Gen: proto.FirstGeneration, Role: owner, Box: *historicalBox,
	}}
	var openedHistorical proto.SharedKeySeed
	if err := core.OpenSecretBoxInto(&openedHistorical, *historicalBox, &historicalBoxKey); err != nil {
		return err
	}
	if openedHistorical.Gen != proto.FirstGeneration || !bytes.Equal(openedHistorical.Seed[:], pukSeed[:]) {
		return fmt.Errorf("official PUK seed-chain unbox returned the wrong generation")
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

	teamName := proto.Name("fixtureteam").Normalize()
	teamNameCommitment := rem.NameCommitment{Name: teamName, Seq: proto.FirstNameSeqno}
	teamRoles := teamlib.EldestRoles()
	teamKeys := make([]core.SharedPrivateSuiter, 0, len(teamRoles))
	teamSeeds := make([]proto.SecretSeed32, 0, len(teamRoles))
	teamHEPKs := proto.HEPKSet{}
	for roleIndex, role := range teamRoles {
		var seed proto.SecretSeed32
		for i := range seed {
			seed[i] = byte(31*roleIndex + i + 7)
		}
		ptk, err := core.NewSharedPrivateSuite25519(
			proto.EntityType_NamedTeam, role, seed, proto.FirstGeneration, hostID,
		)
		if err != nil {
			return err
		}
		hepk, err := ptk.ExportHEPK()
		if err != nil {
			return err
		}
		teamHEPKs.V = append(teamHEPKs.V, *hepk)
		teamKeys = append(teamKeys, ptk)
		teamSeeds = append(teamSeeds, seed)
	}
	var removalCommitment proto.KeyCommitment
	for i := range removalCommitment {
		removalCommitment[i] = byte(201 + i)
	}
	ownerKey := proto.KeyOwner{Party: uid.ToPartyID(), SrcRole: owner}
	teamEldest, err := teamlib.MakeEldestLink(
		hostID, &teamNameCommitment, ownerKey, rotatedPuk, teamKeys,
		*trustedTreeRoot,
		&removalCommitment, nil,
	)
	if err != nil {
		return err
	}
	teamID, err := teamEldest.TeamID.ToTeamID()
	if err != nil {
		return err
	}
	teamHEPKSet, err := core.ImportHEPKSet(&teamHEPKs)
	if err != nil {
		return err
	}
	if _, err := teamlib.OpenEldestLink(teamEldest.Link, teamHEPKSet, hostID); err != nil {
		return fmt.Errorf("official team eldest verification: %w", err)
	}
	teamHash, err := core.LinkHash(teamEldest.Link)
	if err != nil {
		return err
	}
	teamInput := proto.MerkleTreeRFInput{
		Ct: proto.ChainType_Team, Entity: teamID.EntityID(), Seqno: proto.ChainEldestSeqno,
	}
	var teamKey proto.MerkleTreeRFOutput
	if err := merkle.KeyHash(&teamKey, teamInput); err != nil {
		return err
	}
	teamNameID, err := merkle.NameToEntityID(teamName, hostID)
	if err != nil {
		return err
	}
	teamNameFirst, err := merkle.HashName(teamNameID, proto.FirstNameSeqno)
	if err != nil {
		return err
	}
	teamNameNext, err := merkle.HashName(teamNameID, proto.FirstNameSeqno+1)
	if err != nil {
		return err
	}
	teamNameValue := rem.NewEntityIDMerkleValueWithV1(teamID.EntityID())
	teamNameLeaf, err := core.PrefixedHash(&teamNameValue)
	if err != nil {
		return err
	}
	teamEntries := []fixtureMerkleEntry{
		fixtureMerkleEntry{key: *teamNameFirst, value: *teamNameLeaf},
		fixtureMerkleEntry{key: teamKey, value: teamHash.ToStdHash()},
	}
	nextTeamInput := teamInput
	nextTeamInput.Seqno++
	nextTeamInput.Location = teamEldest.NextTreeLocation
	var nextTeamKey proto.MerkleTreeRFOutput
	if err := merkle.KeyHash(&nextTeamKey, nextTeamInput); err != nil {
		return err
	}
	teamQueries := []proto.MerkleTreeRFOutput{*teamNameFirst, *teamNameNext, teamKey, nextTeamKey}
	teamRootNode, teamCompressedPaths, err := fixtureTree(teamEntries, teamQueries)
	if err != nil {
		return err
	}
	teamRoot, _, teamBackPointers, err := fixtureMerkleRoot(
		trustedV1.Epno+1, teamRootNode, trustedV1.Hostchain, hashes,
	)
	if err != nil {
		return err
	}
	_, teamHistoricalEpochs := merkle.MerkleCollectRoots(trustedV1.Epno+1, trustedV1.Epno)
	teamHistoricalEpochs = slices.DeleteFunc(teamHistoricalEpochs, func(epoch proto.MerkleEpno) bool {
		return epoch == trustedV1.Epno
	})
	teamHistoricalHashes := make([]proto.MerkleRootHash, len(teamHistoricalEpochs))
	for i, epoch := range teamHistoricalEpochs {
		teamHistoricalHashes[i] = hashes[epoch]
	}
	teamHistoricalRes := rem.GetHistoricalRootsRes{Hashes: teamHistoricalHashes}
	teamPaths := proto.MerklePathsCompressed{Root: teamRoot, Paths: teamCompressedPaths}
	memberPublic, err := core.PublicizeToSPSBoxer(
		rotatedPuk,
		proto.FQParty{Party: uid.ToPartyID(), Host: hostID},
	)
	if err != nil {
		return err
	}
	teamBoxer, err := core.NewSharedKeyBoxer(hostID, rotatedPuk)
	if err != nil {
		return err
	}
	for _, ptk := range teamKeys {
		if err := teamBoxer.Box(ptk, memberPublic); err != nil {
			return err
		}
	}
	teamBoxSet, err := teamBoxer.Finish()
	if err != nil {
		return err
	}
	teamParcels := make([]proto.SharedKeyParcel, len(teamBoxSet.Boxes))
	senderID, err := rotatedPuk.EntityID()
	if err != nil {
		return err
	}
	for i, box := range teamBoxSet.Boxes {
		teamParcels[i] = proto.SharedKeyParcel{
			Box: box, Sender: senderID, BoxId: teamBoxSet.Id,
			TempDHKeySigned: teamBoxSet.TempDHKeySigned,
		}
	}
	teamChain := rem.TeamChain{
		Links:     []proto.LinkOuter{*teamEldest.Link},
		Locations: []proto.TreeLocation{*teamEldest.NextTreeLocation},
		Teamnames: []rem.NameCommitmentAndKey{{
			Unc: teamNameCommitment, Key: *teamEldest.TeamnameCommitmentKey,
		}},
		Merkle: teamPaths, TeamnameUtf8: proto.NameUtf8("fixtureteam"),
		NumTeamnameLinks: 2, Boxes: teamParcels, Hepks: teamHEPKs,
	}
	viewReq := rem.TeamVOBearerTokenReq{
		Team: proto.FQTeamIDOrName{
			Host:     hostID,
			IdOrName: proto.NewTeamIDOrNameWithTrue(teamID.EntityID()),
		},
		Member:  proto.FQParty{Party: uid.ToPartyID(), Host: hostID},
		SrcRole: owner,
		Gen:     proto.FirstGeneration + 1,
	}
	viewChallengeFrame, err := rpcRequestFrame(rem.TeamLoaderProtocolID, 0, viewReq.Export())
	if err != nil {
		return err
	}
	var viewToken rem.TeamVOBearerToken
	var viewKeyID proto.HMACKeyID
	var viewMAC proto.HMAC
	for i := range viewToken {
		viewToken[i] = byte(0x40 + i)
		viewKeyID[i] = byte(0x60 + i)
	}
	for i := range viewMAC {
		viewMAC[i] = byte(0x80 + i)
	}
	viewChallenge := rem.TeamVOBearerTokenChallenge{
		Payload: rem.TeamVOBearerTokenChallengePayload{
			Req: viewReq, Tm: proto.Time(1_700_000_000_000), Tok: viewToken, Id: viewKeyID,
		},
		Mac: viewMAC,
	}
	viewSignature, err := rotatedPuk.Sign(&viewChallenge)
	if err != nil {
		return err
	}
	viewActivateArg := rem.ActivateTeamVOBearerTokenArg{Ch: viewChallenge, Sig: *viewSignature}
	viewActivateFrame, err := rpcRequestFrame(rem.TeamLoaderProtocolID, 1, viewActivateArg.Export())
	if err != nil {
		return err
	}
	teamLoadArg := rem.LoadTeamChainArg{
		Team: proto.FQTeam{Team: teamID, Host: hostID},
		Tok:  rem.NewTokenVariantWithTeamvobearer(viewToken), Start: proto.ChainEldestSeqno,
	}
	teamLoadFrame, err := rpcRequestFrame(rem.TeamLoaderProtocolID, 3, teamLoadArg.Export())
	if err != nil {
		return err
	}

	// Build a deterministic read-only KV tree with the official v0.1.9
	// implementation. The member-min PTK is deliberately used so these
	// fixtures exercise the same role/generation lookup as a real team vault.
	kvRole := teamRoles[0]
	kvGeneration := proto.FirstGeneration
	appDerivation := proto.NewAppKeyDerivationWithEnum(proto.AppKeyEnum_KVStore)
	kvSeed, err := core.GenericDeriveKey32(teamKeys[0].AppKey(), &appDerivation)
	if err != nil {
		return err
	}
	kvKeys := kv.NewKeyBundle(kvSeed)
	var rootDirID proto.DirID
	var rootDirSeed proto.DirKeySeed
	for i := range rootDirID {
		rootDirID[i] = byte(0xa0 + i)
		rootDirSeed[i] = byte(0x20 + i)
	}
	dirKeys := kv.NewKeyBundle((*proto.SecretSeed32)(&rootDirSeed))
	dirSeedCiphertext, err := kvKeys.BoxWithNonce(&rootDirSeed, rootDirID.ToNonce())
	if err != nil {
		return err
	}
	roleAndGen := proto.RoleAndGen{Role: kvRole, Gen: kvGeneration}
	kvDir := proto.KVDir{
		Id: rootDirID, Version: 1,
		Box:       proto.SeedBoxExternalNonce{Rg: roleAndGen, Ctext: dirSeedCiphertext},
		WriteRole: proto.AdminRole, Status: proto.KVDirStatus_Active,
	}
	kvDirPair := proto.KVDirPair{Active: kvDir}
	kvParty := proto.FQParty{Party: teamID.ToPartyID(), Host: hostID}
	kvRoot := proto.KVRoot{Root: rootDirID, Vers: 1, Rg: roleAndGen}
	kvRoot.BindingMac, err = func() (proto.HMAC, error) {
		mac, err := kvKeys.Hmac(kvRoot.ToBindingPayload(kvParty))
		if err != nil {
			return proto.HMAC{}, err
		}
		return *mac, nil
	}()
	if err != nil {
		return err
	}

	makeDirent := func(index byte, name string, value proto.KVNodeID) (proto.KVDirent, error) {
		var id proto.DirentID
		for i := range id {
			id[i] = index + byte(i)
		}
		namePayload := lcl.KVDirentNamePayload{
			ParentDir: rootDirID, DirVersion: 1, Name: proto.KVPathComponent(name),
		}
		nameBox, err := dirKeys.Box(&namePayload)
		if err != nil {
			return proto.KVDirent{}, err
		}
		nameMAC, err := dirKeys.Hmac(&namePayload)
		if err != nil {
			return proto.KVDirent{}, err
		}
		ret := proto.KVDirent{
			ParentDir: rootDirID, Id: id, Value: value, Version: 1, DirVersion: 1,
			WriteRole: proto.AdminRole, NameMac: *nameMAC, NameBox: *nameBox,
			DirStatus: proto.KVDirStatus_Active, Ctime: proto.TimeMicro(1_700_000_000_000_000 + int64(index)),
		}
		binding, err := dirKeys.Hmac(ret.ToBindingPayload())
		if err != nil {
			return proto.KVDirent{}, err
		}
		ret.BindingMac = *binding
		return ret, nil
	}

	var smallID proto.SmallFileID
	var symlinkID proto.SymlinkID
	var largeID proto.FileID
	for i := range smallID {
		smallID[i] = byte(0x40 + i)
		symlinkID[i] = byte(0x60 + i)
		largeID[i] = byte(0x80 + i)
	}
	smallPayload := lcl.NewSmallFileBoxPayloadWithSmallfile(lcl.SmallFileData("fixture small secret\n"))
	smallCiphertext, err := kvKeys.BoxPaddedWithNonce(&smallPayload, smallID.NaclNonce())
	if err != nil {
		return err
	}
	smallBox := proto.SmallFileBox{Rg: roleAndGen, DataBox: smallCiphertext}
	symlinkPayload := lcl.NewSmallFileBoxPayloadWithSymlink(proto.KVPath("/small.txt"))
	symlinkCiphertext, err := kvKeys.BoxPaddedWithNonce(&symlinkPayload, symlinkID.NaclNonce())
	if err != nil {
		return err
	}
	symlinkBox := proto.SmallFileBox{Rg: roleAndGen, DataBox: symlinkCiphertext}

	var fileSeed proto.FileKeySeed
	for i := range fileSeed {
		fileSeed[i] = byte(0xc0 + i)
	}
	fileSeedPayload := lcl.FileKeyBoxPayload{Id: largeID, Vers: 1, Seed: fileSeed}
	fileSeedBox, err := kvKeys.Box(&fileSeedPayload)
	if err != nil {
		return err
	}
	largeMetadata := proto.LargeFileMetadata{Rg: roleAndGen, KeySeed: *fileSeedBox, Vers: 1}
	largePlaintext := proto.ChunkPlaintext("fixture large-file chunk\n")
	chunkNoncePayload := lcl.ChunkNoncePayload{Id: largeID, Offset: 0, Final: true}
	chunkHash, err := core.PrefixedHash(&chunkNoncePayload)
	if err != nil {
		return err
	}
	var chunkNonce [24]byte
	copy(chunkNonce[:], chunkHash[:24])
	encodedChunk, err := core.EncodeToBytes(&largePlaintext)
	if err != nil {
		return err
	}
	paddedChunk, err := kv.PadChunk(encodedChunk)
	if err != nil {
		return err
	}
	encryptedChunk := proto.Chunk(secretbox.Seal(nil, paddedChunk, &chunkNonce, (*[32]byte)(&fileSeed)))
	chunkResponse := rem.GetEncryptedChunkRes{Chunk: encryptedChunk, Offset: 0, Final: true}

	smallDirent, err := makeDirent(0x10, "small.txt", smallID.KVNodeID())
	if err != nil {
		return err
	}
	symlinkNodeID := proto.KVNodeID{byte(proto.KVNodeType_Symlink)}
	copy(symlinkNodeID[1:], symlinkID[:])
	symlinkDirent, err := makeDirent(0x30, "latest", symlinkNodeID)
	if err != nil {
		return err
	}
	largeDirent, err := makeDirent(0x50, "large.bin", largeID.KVNodeID())
	if err != nil {
		return err
	}
	kvList := rem.KVListRes{
		Ents: []proto.KVDirent{smallDirent, symlinkDirent, largeDirent}, Final: true,
		ExtEnts: []proto.KVExtendedDirent{{Pos: 0, Sfb: smallBox}},
	}
	kvAuth := rem.NewKVAuthWithTeam(viewToken)
	kvSelectFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 18, (&rem.KVSelectVhost{Host: hostID}).Export(), 0)
	if err != nil {
		return err
	}
	kvSelectResponseFrame, err := rpcVoidResponseFrame(0)
	if err != nil {
		return err
	}
	kvRootFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 8, (&rem.KvGetRootArg{Auth: kvAuth}).Export(), 1)
	if err != nil {
		return err
	}
	kvDirFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 12, (&rem.KvGetDirArg{Auth: kvAuth, Id: rootDirID}).Export(), 1)
	if err != nil {
		return err
	}
	kvListFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 14, (&rem.KvListArg{
		Auth: kvAuth, Dir: rootDirID,
		Opts: rem.KVListOpts{Start: proto.NewKVListPaginationWithNone(), Num: 100, LoadSmallFiles: true},
	}).Export(), 1)
	if err != nil {
		return err
	}
	kvSmallNodeFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 10, (&rem.KvGetNodeArg{Auth: kvAuth, Id: smallID.KVNodeID()}).Export(), 1)
	if err != nil {
		return err
	}
	kvLargeNodeFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 10, (&rem.KvGetNodeArg{Auth: kvAuth, Id: largeID.KVNodeID()}).Export(), 1)
	if err != nil {
		return err
	}
	kvChunkFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 11, (&rem.KvGetEncryptedChunkArg{Auth: kvAuth, Id: largeID, Offset: 0}).Export(), 1)
	if err != nil {
		return err
	}
	kvVersions := proto.PathVersionVector{Root: kvRoot.Vers, Path: []proto.DirVersion{{
		Id: rootDirID, Vers: kvDir.Version, De: []proto.DirentVersion{
			{Id: smallDirent.Id, Vers: smallDirent.Version},
			{Id: symlinkDirent.Id, Vers: symlinkDirent.Version},
			{Id: largeDirent.Id, Vers: largeDirent.Version},
		},
	}}}
	kvCacheFrame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, 13, (&rem.KvCacheCheckArg{Req: rem.KVReqHeader{
		Auth: kvAuth, Precondition: &kvVersions,
	}}).Export(), 1)
	if err != nil {
		return err
	}
	kvStaleFrame, err := rpcErrorResponseFrame(1, proto.NewStatusWithKvStaleCacheError(kvVersions))
	if err != nil {
		return err
	}

	// Mutation fixtures use separately fixed name-box and lock nonces so their
	// exact bytes remain stable across fixture regeneration.
	writeDirent := smallDirent
	for i := range writeDirent.Id {
		writeDirent.Id[i] = byte(0xd0 + i)
	}
	writeDirent.Ctime = 1_700_000_000_000_777
	writeNamePayload := lcl.KVDirentNamePayload{
		ParentDir: rootDirID, DirVersion: 1, Name: proto.KVPathComponent("write.txt"),
	}
	var writeNameNonce proto.NaclNonce
	for i := range writeNameNonce {
		writeNameNonce[i] = byte(0xe0 + i)
	}
	writeNameCiphertext, err := dirKeys.BoxPaddedWithNonce(&writeNamePayload, &writeNameNonce)
	if err != nil {
		return err
	}
	writeDirent.NameBox = proto.NewSecretBoxWithNacl(proto.NaclSecretBox{
		Nonce: writeNameNonce, Ciphertext: writeNameCiphertext,
	})
	writeNameMAC, err := dirKeys.Hmac(&writeNamePayload)
	if err != nil {
		return err
	}
	writeDirent.NameMac = *writeNameMAC
	writeBinding, err := dirKeys.Hmac(writeDirent.ToBindingPayload())
	if err != nil {
		return err
	}
	writeDirent.BindingMac = *writeBinding
	writeHeader := rem.KVReqHeader{Auth: kvAuth, Precondition: &kvVersions}
	uploadChunk := proto.UploadChunk{
		Data: proto.NaclCiphertext(encryptedChunk), Offset: 0,
		Final: &proto.UploadFinal{Sz: proto.Size(len(encryptedChunk))},
	}
	var writeFileSeedNonce proto.NaclNonce
	for i := range writeFileSeedNonce {
		writeFileSeedNonce[i] = byte(0xb0 + i)
	}
	writeFileSeedCiphertext, err := kvKeys.BoxWithNonce(&fileSeedPayload, &writeFileSeedNonce)
	if err != nil {
		return err
	}
	writeLargeMetadata := proto.LargeFileMetadata{
		Rg: roleAndGen,
		KeySeed: proto.NewSecretBoxWithNacl(proto.NaclSecretBox{
			Nonce: writeFileSeedNonce, Ciphertext: writeFileSeedCiphertext,
		}),
		Vers: 1,
	}
	var lockID rem.LockID
	for i := range lockID {
		lockID[i] = byte(0xf0 + i)
	}
	lock := rem.KVLock{
		Idp:    proto.KVDirentIDPair{ParentDirID: rootDirID, DirentID: writeDirent.Id},
		LockID: lockID,
	}
	writeFrames := []struct {
		name   string
		method rpc.Position
		arg    any
	}{
		{"kv-mkdir-request.frame", 0, (&rem.KvMkdirArg{Hdr: writeHeader, Dir: kvDir}).Export()},
		{"kv-put-request.frame", 1, (&rem.KvPutArg{Hdr: writeHeader, Dirents: []proto.KVDirent{writeDirent}}).Export()},
		{"kv-put-root-request.frame", 2, (&rem.KvPutRootArg{Auth: kvAuth, Root: kvRoot}).Export()},
		{"kv-file-upload-init-request.frame", 3, (&rem.KvFileUploadInitArg{Auth: kvAuth, FileID: largeID, Md: writeLargeMetadata, Chunk: uploadChunk}).Export()},
		{"kv-file-upload-chunk-request.frame", 4, (&rem.KvFileUploadChunkArg{Auth: kvAuth, FileID: largeID, Chunk: uploadChunk}).Export()},
		{"kv-put-small-request.frame", 7, (&rem.KvPutSmallFileOrSymlinkArg{Auth: kvAuth, Id: smallID.KVNodeID(), Sfb: smallBox}).Export()},
		{"kv-lock-acquire-request.frame", 15, (&rem.KvLockAcquireArg{Auth: kvAuth, Lock: lock, Timeout: 1000}).Export()},
		{"kv-lock-release-request.frame", 16, (&rem.KvLockReleaseArg{Auth: kvAuth, Lock: lock}).Export()},
	}
	kvWriteFrames := make(map[string][]byte, len(writeFrames))
	for _, item := range writeFrames {
		frame, err := rpcRequestFrameAt(rem.KVStoreProtocolID, item.method, item.arg, 1)
		if err != nil {
			return err
		}
		kvWriteFrames[item.name] = frame
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
	memberPUKArg := rem.GetPUKForRoleArg{
		Role: proto.NewRoleWithMember(7), TargetPublicKeyId: deviceID.EntityID(),
	}
	memberPUKFrame, err := rpcRequestFrame(rem.UserProtocolID, 14, memberPUKArg.Export())
	if err != nil {
		return err
	}
	certArg := rem.GetClientCertChainArg{Uid: uid, Key: deviceID.EntityID()}
	regSelectFrame, err := rpcRequestFrameAt(rem.RegProtocolID, 15, (&rem.RegSelectVhost{Host: hostID}).Export(), 0)
	if err != nil {
		return err
	}
	regSelectResponseFrame, err := rpcVoidResponseFrame(0)
	if err != nil {
		return err
	}
	certFrame, err := rpcRequestFrameAt(rem.RegProtocolID, 1, certArg.Export(), 1)
	if err != nil {
		return err
	}
	merkleSelectFrame, err := rpcRequestFrameAt(rem.MerkleQueryProtocolID, 8, (&rem.MerkleSelectVHostArg{Host: hostID}).Export(), 0)
	if err != nil {
		return err
	}
	merkleSelectResponseFrame, err := rpcVoidResponseFrame(0)
	if err != nil {
		return err
	}
	currentRootFrame, err := rpcRequestFrameAt(
		rem.MerkleQueryProtocolID,
		2,
		(&rem.GetCurrentRootArg{HostID: &hostID}).Export(),
		1,
	)
	if err != nil {
		return err
	}
	historicalFull, historicalHashEpochs := merkle.MerkleCollectRoots(trustedV1.Epno+3, trustedV1.Epno)
	historicalFull = historicalFull[1:]
	historicalHashEpochs = slices.DeleteFunc(historicalHashEpochs, func(epoch proto.MerkleEpno) bool {
		return epoch == trustedV1.Epno
	})
	historicalArg := rem.GetHistoricalRootsArg{HostID: &hostID, Full: historicalFull, Hashes: historicalHashEpochs}
	historicalFrame, err := rpcRequestFrameAt(rem.MerkleQueryProtocolID, 1, historicalArg.Export(), 1)
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
		{"team-id.snowp", &teamID},
		{"team-eldest-link.snowp", teamEldest.Link},
		{"team-chain.snowp", &teamChain},
		{"team-hepk-set.snowp", &teamHEPKs},
		{"team-merkle-root-996.snowp", &teamRoot},
		{"team-merkle-back-pointers-996.snowp", &teamBackPointers},
		{"team-merkle-historical-response.snowp", &teamHistoricalRes},
		{"team-view-challenge.snowp", &viewChallenge},
		{"kv-root.snowp", &kvRoot},
		{"kv-root-dir.snowp", &kvDirPair},
		{"kv-list.snowp", &kvList},
		{"kv-small-box.snowp", &smallBox},
		{"kv-symlink-box.snowp", &symlinkBox},
		{"kv-large-metadata.snowp", &largeMetadata},
		{"kv-large-chunk.snowp", &chunkResponse},
		{"kv-path-version-vector.snowp", &kvVersions},
		{"kv-write-dirent.snowp", &writeDirent},
		{"kv-write-large-metadata.snowp", &writeLargeMetadata},
		{"kv-upload-chunk.snowp", &uploadChunk},
		{"kv-small-node.snowp", func() core.Encodeable { v := rem.NewKVGetNodeResWithSmallfile(smallBox); return &v }()},
		{"kv-symlink-node.snowp", func() core.Encodeable { v := rem.NewKVGetNodeResWithSymlink(symlinkBox); return &v }()},
		{"kv-large-node.snowp", func() core.Encodeable { v := rem.NewKVGetNodeResWithFile(largeMetadata); return &v }()},
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
		{"team-ptk-member-min-seed.bin", teamSeeds[0][:]},
		{"team-ptk-member-seed.bin", teamSeeds[1][:]},
		{"team-ptk-admin-seed.bin", teamSeeds[2][:]},
		{"team-ptk-owner-seed.bin", teamSeeds[3][:]},
		{"reg-cert-request.frame", certFrame},
		{"reg-select-vhost-request.frame", regSelectFrame},
		{"reg-select-vhost-response.frame", regSelectResponseFrame},
		{"user-load-request.frame", loadFrame},
		{"user-puk-request.frame", pukFrame},
		{"user-member-puk-request.frame", memberPUKFrame},
		{"merkle-current-root-request.frame", currentRootFrame},
		{"merkle-historical-roots-request.frame", historicalFrame},
		{"merkle-select-vhost-request.frame", merkleSelectFrame},
		{"merkle-select-vhost-response.frame", merkleSelectResponseFrame},
		{"team-view-challenge-request.frame", viewChallengeFrame},
		{"team-view-activate-request.frame", viewActivateFrame},
		{"team-load-request.frame", teamLoadFrame},
		{"kv-file-seed.bin", fileSeed[:]},
		{"kv-root-dir-seed.bin", rootDirSeed[:]},
		{"kv-small-plaintext.bin", []byte(smallPayload.Smallfile())},
		{"kv-symlink-plaintext.bin", []byte(symlinkPayload.Symlink())},
		{"kv-large-plaintext.bin", []byte(largePlaintext)},
		{"kv-select-vhost-request.frame", kvSelectFrame},
		{"kv-select-vhost-response.frame", kvSelectResponseFrame},
		{"kv-get-root-request.frame", kvRootFrame},
		{"kv-get-dir-request.frame", kvDirFrame},
		{"kv-list-request.frame", kvListFrame},
		{"kv-get-small-node-request.frame", kvSmallNodeFrame},
		{"kv-get-large-node-request.frame", kvLargeNodeFrame},
		{"kv-get-large-chunk-request.frame", kvChunkFrame},
		{"kv-cache-check-request.frame", kvCacheFrame},
		{"kv-stale-cache-response.frame", kvStaleFrame},
	}
	for name, frame := range kvWriteFrames {
		raw = append(raw, struct {
			name string
			data []byte
		}{name, frame})
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
	kvFiles := make([]fixture, 0)
	for _, file := range append(append([]fixture{}, w.files...), w.rpcFiles...) {
		if len(file.File) >= 3 && file.File[:3] == "kv-" {
			kvFiles = append(kvFiles, file)
		}
	}
	kvManifest := struct {
		Format            string    `json:"format"`
		FOKSVersion       string    `json:"foks_version"`
		OfficialGenerated bool      `json:"official_generated"`
		Files             []fixture `json:"files"`
	}{
		Format:            "foks-v0.1.9-kv-fixtures-v1",
		FOKSVersion:       "v0.1.9",
		OfficialGenerated: true,
		Files:             kvFiles,
	}
	kvEncoded, err := json.MarshalIndent(kvManifest, "", "  ")
	if err != nil {
		return err
	}
	kvEncoded = append(kvEncoded, '\n')
	if err := os.WriteFile(filepath.Join(output, "kv-manifest.json"), kvEncoded, 0o644); err != nil {
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
