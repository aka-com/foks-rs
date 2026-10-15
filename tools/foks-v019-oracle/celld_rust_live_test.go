package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/kv"
	"github.com/foks-proj/go-foks/proto/lcl"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
	"os"
	"reflect"
	"testing"
	"time"
)

// Provision against native FOKS; then reconnect unchanged Go keys/certificates
// after the controller has replaced native storage with the celld authority.
func TestGoClientAgainstCelldProjection(t *testing.T) {
	ready, continuation := os.Getenv("FOKS_GO_CELLD_READY_PATH"), os.Getenv("FOKS_GO_CELLD_CONTINUE_PATH")
	if ready == "" || continuation == "" {
		t.Skip("requires celld proof controller")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 110*time.Second)
	defer cancel()
	address := os.Getenv("FOKS_GO_RUST_PROBE")
	roots := liveRootPool(t, os.Getenv("FOKS_GO_RUST_CA_DER"))
	probeRPC, closeProbe := liveRPCClient(t, ctx, address, roots, nil)
	probeClient := core.NewProbeClient(probeRPC, nil)
	probe, err := probeClient.Probe(ctx, rem.ProbeArg{Hostname: proto.Hostname("localhost")})
	if err != nil {
		t.Fatal(err)
	}
	chain, err := core.PlayChain(proto.TCPAddr(address), probe.Hostchain, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := verifyMerkleRoot(chain, probe.MerkleRoot); err != nil {
		t.Fatal(err)
	}
	zone, err := core.CheckZoneSig(*chain, probe)
	if err != nil {
		t.Fatal(err)
	}
	serviceRoots, err := chain.RootCACertPool()
	if err != nil {
		t.Fatal(err)
	}
	publicRPC, closePublic := liveRPCClient(t, ctx, string(zone.Services.Reg), serviceRoots, nil)
	regClient := core.NewRegClient(publicRPC, nil)
	merkleClient := core.NewMerkleQueryClient(publicRPC, nil)
	user := liveSignup(t, ctx, &regClient, &merkleClient, chain.HostID())
	deviceID, err := user.device.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	certChain, err := regClient.GetClientCertChain(ctx, rem.GetClientCertChainArg{Uid: user.uid, Key: deviceID})
	if err != nil {
		t.Fatal(err)
	}
	key, err := user.device.PrivateKeyForCert()
	if err != nil {
		t.Fatal(err)
	}
	certificate := &tls.Certificate{Certificate: certChain, PrivateKey: key}
	authRPC, closeAuth := liveRPCClient(t, ctx, string(zone.Services.User), serviceRoots, certificate)
	perform := celldPreparedKV(t, ctx, authRPC, &user)
	closeAuth()
	closePublic()
	closeProbe()
	marker, err := os.OpenFile(ready, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = marker.WriteString("native identity and KV root provisioned\n"); err != nil {
		t.Fatal(err)
	}
	if err = marker.Close(); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(45 * time.Second)
	for {
		if _, err = os.Stat(continuation); err == nil {
			break
		} else if !os.IsNotExist(err) {
			t.Fatal(err)
		}
		if time.Now().After(deadline) {
			t.Fatal("celld import controller timed out")
		}
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case <-time.After(20 * time.Millisecond):
		}
	}
	authRPC, closeAuth = liveRPCClient(t, ctx, string(zone.Services.User), serviceRoots, certificate)
	defer closeAuth()
	perform(authRPC)
	t.Log("unchanged pinned Go mTLS client put/get/decrypt and exact replay completed against celld authority")
}

func celldPreparedKV(
	t *testing.T,
	ctx context.Context,
	authRPC *rpc.Client,
	user *goLiveUser,
) func(*rpc.Client) {
	t.Helper()
	client := core.NewKVStoreClient(authRPC, nil)
	auth := rem.KVAuth{}
	derivation := proto.NewAppKeyDerivationWithEnum(proto.AppKeyEnum_KVStore)
	seed, err := core.GenericDeriveKey32(user.puk.AppKey(), &derivation)
	if err != nil {
		t.Fatal(err)
	}
	rootKeys := kv.NewKeyBundle(seed)
	var rootID proto.DirID
	var rootSeed proto.DirKeySeed
	for index := range rootID {
		rootID[index] = byte(0xa0 + index)
		rootSeed[index] = byte(0x40 + index)
	}
	rootCiphertext, err := rootKeys.BoxWithNonce(&rootSeed, rootID.ToNonce())
	if err != nil {
		t.Fatal(err)
	}
	roleAndGeneration := proto.RoleAndGen{Role: proto.OwnerRole, Gen: proto.FirstGeneration}
	directory := proto.KVDir{
		Id: rootID, Version: 1,
		Box:       proto.SeedBoxExternalNonce{Rg: roleAndGeneration, Ctext: rootCiphertext},
		WriteRole: proto.OwnerRole, Status: proto.KVDirStatus_Active,
	}
	root := proto.KVRoot{Root: rootID, Vers: 1, Rg: roleAndGeneration}
	rootMAC, err := rootKeys.Hmac(root.ToBindingPayload(proto.FQParty{
		Party: user.uid.ToPartyID(), Host: user.host,
	}))
	if err != nil {
		t.Fatal(err)
	}
	root.BindingMac = *rootMAC
	if err := client.KvMkdir(ctx, rem.KvMkdirArg{Hdr: rem.KVReqHeader{Auth: auth}, Dir: directory}); err != nil {
		t.Fatalf("Go KV mkdir: %v", err)
	}
	if err := client.KvPutRoot(ctx, rem.KvPutRootArg{Auth: auth, Root: root}); err != nil {
		t.Fatalf("Go KV root put: %v", err)
	}
	return func(authRPC *rpc.Client) {
		client := core.NewKVStoreClient(authRPC, nil)
		directoryKeys := kv.NewKeyBundle((*proto.SecretSeed32)(&rootSeed))
		var smallID proto.SmallFileID
		for index := range smallID {
			smallID[index] = byte(0x70 + index)
		}
		payload := lcl.NewSmallFileBoxPayloadWithSmallfile(lcl.SmallFileData("go client secret\n"))
		ciphertext, err := rootKeys.BoxPaddedWithNonce(&payload, smallID.NaclNonce())
		if err != nil {
			t.Fatal(err)
		}
		smallBox := proto.SmallFileBox{Rg: roleAndGeneration, DataBox: ciphertext}
		if err := client.KvPutSmallFileOrSymlink(ctx, rem.KvPutSmallFileOrSymlinkArg{
			Auth: auth, Id: smallID.KVNodeID(), Sfb: smallBox,
		}); err != nil {
			t.Fatalf("Go KV small-file put: %v", err)
		}
		namePayload := lcl.KVDirentNamePayload{
			ParentDir: rootID, DirVersion: 1, Name: proto.KVPathComponent("secret.txt"),
		}
		nameBox, err := directoryKeys.Box(&namePayload)
		if err != nil {
			t.Fatal(err)
		}
		nameMAC, err := directoryKeys.Hmac(&namePayload)
		if err != nil {
			t.Fatal(err)
		}
		var direntID proto.DirentID
		for index := range direntID {
			direntID[index] = byte(0xc0 + index)
		}
		dirent := proto.KVDirent{
			ParentDir: rootID, Id: direntID, Value: smallID.KVNodeID(), Version: 1, DirVersion: 1,
			WriteRole: proto.OwnerRole, NameMac: *nameMAC, NameBox: *nameBox, DirStatus: proto.KVDirStatus_Active,
		}
		binding, err := directoryKeys.Hmac(dirent.ToBindingPayload())
		if err != nil {
			t.Fatal(err)
		}
		dirent.BindingMac = *binding
		if err := client.KvPut(ctx, rem.KvPutArg{
			Hdr: rem.KVReqHeader{Auth: auth}, Dirents: []proto.KVDirent{dirent},
		}); err != nil {
			t.Fatalf("Go KV dirent put: %v", err)
		}
		got, err := client.KvGet(ctx, rem.KvGetArg{
			Hdr: rem.KVReqHeader{Auth: auth},
			Path: rem.KVNodePathMultiple{
				ParentDir: rootID,
				Names:     []rem.KVNameMACAtDirVersion{{DirVers: 1, Mac: *nameMAC}},
			},
			Follow: rem.FollowBehavior_Any,
		})
		if err != nil {
			t.Fatalf("Go KV composed get: %v", err)
		}
		if got.Data == nil {
			t.Fatal("Go KV composed get returned no data")
		}
		typ, err := got.Data.GetT()
		if err != nil || typ != proto.KVNodeType_SmallFile {
			t.Fatalf("Go KV composed get type=%v err=%v", typ, err)
		}
		if actual := got.Data.Smallfile(); !bytes.Equal(actual.DataBox, smallBox.DataBox) {
			t.Fatal("Go KV composed get returned the wrong small-file ciphertext")
		}
		if _, err := client.KvUsage(ctx, auth); err != nil {
			t.Fatalf("Go KV usage: %v", err)
		}
		var opened lcl.SmallFileBoxPayload
		if err := rootKeys.Unbox(&opened, proto.NewSecretBoxWithNacl(proto.NaclSecretBox{Nonce: *smallID.NaclNonce(), Ciphertext: got.Data.Smallfile().DataBox})); err != nil {
			t.Fatalf("Go decrypt celld-backed file: %v", err)
		}
		if !bytes.Equal(opened.Smallfile(), payload.Smallfile()) {
			t.Fatal("Go decrypted plaintext differs")
		}
		before, err := client.KvUsage(ctx, auth)
		if err != nil {
			t.Fatal(err)
		}
		if err := client.KvPutSmallFileOrSymlink(ctx, rem.KvPutSmallFileOrSymlinkArg{Auth: auth, Id: smallID.KVNodeID(), Sfb: smallBox}); err != nil {
			t.Fatal(err)
		}
		if err := client.KvPut(ctx, rem.KvPutArg{Hdr: rem.KVReqHeader{Auth: auth}, Dirents: []proto.KVDirent{dirent}}); err != nil {
			t.Fatal(err)
		}
		after, err := client.KvUsage(ctx, auth)
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(before, after) {
			t.Fatalf("exact replay changed usage: before=%+v after=%+v", before, after)
		}
	}

}
