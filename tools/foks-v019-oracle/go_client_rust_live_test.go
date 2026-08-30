package main

import (
	"bytes"
	"context"
	"crypto/sha512"
	"crypto/tls"
	"crypto/x509"
	"net"
	"os"
	"testing"
	"time"

	"github.com/foks-proj/go-foks/client/libclient"
	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/kv"
	"github.com/foks-proj/go-foks/lib/merkle"
	teamlib "github.com/foks-proj/go-foks/lib/team"
	"github.com/foks-proj/go-foks/proto/lcl"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
)

type goLiveUser struct {
	host      proto.HostID
	uid       proto.UID
	device    core.PrivateSuiter
	puk       core.SharedPrivateSuiter
	prev      proto.LinkHash
	nextSeqno proto.Seqno
}

func TestGoClientAgainstRustServer(t *testing.T) {
	probeAddress := os.Getenv("FOKS_GO_RUST_PROBE")
	caPath := os.Getenv("FOKS_GO_RUST_CA_DER")
	if probeAddress == "" || caPath == "" {
		t.Skip("run through run-go-client-compat.sh")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	roots := liveRootPool(t, caPath)

	probeRPC, closeProbe := liveRPCClient(t, ctx, probeAddress, roots, nil)
	defer closeProbe()
	probeClient := core.NewProbeClient(probeRPC, nil)
	probe, err := probeClient.Probe(ctx, rem.ProbeArg{Hostname: proto.Hostname("localhost")})
	if err != nil {
		t.Fatalf("official probe: %v", err)
	}
	chain, err := core.PlayChain(proto.TCPAddr(probeAddress), probe.Hostchain, nil)
	if err != nil {
		t.Fatalf("play Rust hostchain: %v", err)
	}
	if _, err := verifyMerkleRoot(chain, probe.MerkleRoot); err != nil {
		t.Fatalf("verify Rust bootstrap Merkle root: %v", err)
	}
	zone, err := core.CheckZoneSig(*chain, probe)
	if err != nil {
		t.Fatalf("verify Rust public zone: %v", err)
	}
	publicAddress := string(zone.Services.Reg)
	authenticatedAddress := string(zone.Services.User)
	host := chain.HostID()
	serviceRoots, err := chain.RootCACertPool()
	if err != nil {
		t.Fatalf("load delegated TLS roots from Rust hostchain: %v", err)
	}

	publicRPC, closePublic := liveRPCClient(t, ctx, publicAddress, serviceRoots, nil)
	defer closePublic()
	regClient := core.NewRegClient(publicRPC, nil)
	merkleClient := core.NewMerkleQueryClient(publicRPC, nil)
	liveKexRelay(t, ctx, publicRPC, host)
	if id, err := regClient.JoinWaitList(ctx, proto.Email("go-client@example.com")); err != nil || id[0] != 1 {
		t.Fatalf("official waitlist registration: id=%v err=%v", id, err)
	}
	liveLogSend(t, ctx, publicRPC, "public-client.log")
	config, err := regClient.GetServerConfig(ctx)
	if err != nil {
		t.Fatalf("registration config: %v", err)
	}
	if config.Typ != proto.HostType_Standalone {
		t.Fatalf("registration config host type = %v", config.Typ)
	}

	user := liveSignup(t, ctx, &regClient, &merkleClient, host)
	if _, err := regClient.ResolveUsername(ctx, rem.ResolveUsernameArg{
		N: "gocompat", Auth: rem.NewLoadUserChainAuthWithAslocaluser(),
	}); err == nil {
		t.Fatal("public username resolution accepted local-user authorization without a principal")
	}
	resolved, err := regClient.ResolveUsername(ctx, rem.ResolveUsernameArg{
		N: "gocompat", Auth: rem.NewLoadUserChainAuthWithOpenvhost(),
	})
	if err != nil || !resolved.Eq(user.uid) {
		t.Fatalf("resolve signed-up user: uid=%v err=%v", resolved, err)
	}
	deviceID, err := user.device.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	certChain, err := regClient.GetClientCertChain(ctx, rem.GetClientCertChainArg{Uid: user.uid, Key: deviceID})
	if err != nil {
		t.Fatalf("fetch device certificate: %v", err)
	}
	privateKey, err := user.device.PrivateKeyForCert()
	if err != nil {
		t.Fatal(err)
	}
	certificate := &tls.Certificate{Certificate: certChain, PrivateKey: privateKey}

	authRPC, closeAuthenticated := liveRPCClient(t, ctx, authenticatedAddress, serviceRoots, certificate)
	defer closeAuthenticated()
	userClient := core.NewUserClient(authRPC, nil)
	liveLogSend(t, ctx, authRPC, "authenticated-client.log")
	uid, err := userClient.Ping(ctx)
	if err != nil || !uid.Eq(user.uid) {
		t.Fatalf("activate user ping: uid=%v err=%v", uid, err)
	}

	liveProvision(t, ctx, &userClient, &regClient, &merkleClient, &user)
	loadedUser, err := userClient.LoadUserChain(ctx, rem.LoadUserChainArg{
		Uid: user.uid, Start: proto.ChainEldestSeqno, Auth: rem.NewLoadUserChainAuthWithAslocaluser(),
	})
	if err != nil || len(loadedUser.Links) != 2 {
		t.Fatalf("official user-chain load: links=%d err=%v", len(loadedUser.Links), err)
	}
	liveSetPassphrase(t, ctx, &userClient, &merkleClient, &user)
	teamID := liveCreateAndLoadTeam(t, ctx, authRPC, &userClient, &merkleClient, &user)
	liveKVPutGet(t, ctx, authRPC, &user)
	t.Logf("official Go client completed user=%s team=%s", user.uid, teamID)
}

func liveLogSend(t *testing.T, ctx context.Context, client *rpc.Client, name proto.LocalFSPath) {
	t.Helper()
	logClient := core.NewLogSendClient(client, nil)
	id, err := logClient.LogSendInit(ctx)
	if err != nil {
		t.Fatalf("official LogSend init: %v", err)
	}
	payload := rem.LogSendBlob("official Go diagnostic payload")
	hash := proto.StdHash(sha512.Sum512_256(payload))
	if err := logClient.LogSendInitFile(ctx, rem.LogSendInitFileArg{
		Id: id, FileID: 1, Name: name, Len: proto.Size(len(payload)), Hash: hash, NBlocks: 1,
	}); err != nil {
		t.Fatalf("official LogSend file init: %v", err)
	}
	if err := logClient.LogSendUploadBlock(ctx, rem.LogSendUploadBlockArg{
		Id: id, FileID: 1, BlockNo: 0, Block: payload,
	}); err != nil {
		t.Fatalf("official LogSend upload: %v", err)
	}
}

func liveKexRelay(
	t *testing.T,
	ctx context.Context,
	publicRPC *rpc.Client,
	host proto.HostID,
) {
	t.Helper()
	client := rem.KexClient{Cli: publicRPC, ErrorUnwrapper: core.StatusToError}
	var senderSeed, receiverSeed proto.SecretSeed32
	for index := range senderSeed {
		senderSeed[index] = byte(index + 31)
		receiverSeed[index] = byte(index + 131)
	}
	sender, err := core.NewPrivateSuite25519(
		proto.EntityType_Device,
		proto.OwnerRole,
		senderSeed,
		host,
	)
	if err != nil {
		t.Fatal(err)
	}
	receiver, err := core.NewPrivateSuite25519(
		proto.EntityType_Device,
		proto.OwnerRole,
		receiverSeed,
		host,
	)
	if err != nil {
		t.Fatal(err)
	}
	senderPublic, err := sender.EntityPublic()
	if err != nil {
		t.Fatal(err)
	}
	receiverPublic, err := receiver.EntityPublic()
	if err != nil {
		t.Fatal(err)
	}
	var session proto.KexSessionID
	for index := range session {
		session[index] = byte(index + 1)
	}
	var nonce proto.NaclNonce
	for index := range nonce {
		nonce[index] = byte(index + 51)
	}
	wrapper := rem.KexWrapperMsg{
		SessionID: session,
		Sender:    senderPublic.GetEntityID(),
		Seq:       0,
		Payload: proto.NewSecretBoxWithNacl(proto.NaclSecretBox{
			Nonce:      nonce,
			Ciphertext: proto.NaclCiphertext([]byte("official-go-kex-payload")),
		}),
	}
	signature, err := sender.Sign(&wrapper)
	if err != nil {
		t.Fatal(err)
	}
	if err := client.Send(ctx, rem.SendArg{
		Msg: wrapper, Sig: *signature, Actor: rem.KexActorType_Provisioner,
	}); err != nil {
		t.Fatalf("official KEX send: %v", err)
	}
	received, err := client.Receive(ctx, rem.ReceiveArg{
		SessionID: session,
		Receiver:  receiverPublic.GetEntityID(),
		Seq:       0,
		PollWait:  0,
		Actor:     rem.KexActorType_Provisionee,
	})
	if err != nil {
		t.Fatalf("official KEX receive: %v", err)
	}
	if !received.SessionID.Eq(&wrapper.SessionID) ||
		!received.Sender.Eq(wrapper.Sender) ||
		received.Seq != wrapper.Seq ||
		!bytes.Equal(received.Payload.Nacl().Ciphertext, wrapper.Payload.Nacl().Ciphertext) {
		t.Fatalf("official KEX relay changed packet: got=%v want=%v", received, wrapper)
	}
}

func liveRootPool(t *testing.T, path string) *x509.CertPool {
	t.Helper()
	result := x509.NewCertPool()
	der, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	certificate, err := x509.ParseCertificate(der)
	if err != nil {
		t.Fatal(err)
	}
	result.AddCert(certificate)
	return result
}

func liveRPCClient(
	t *testing.T,
	ctx context.Context,
	address string,
	roots *x509.CertPool,
	certificate *tls.Certificate,
) (*rpc.Client, func()) {
	t.Helper()
	config := &tls.Config{RootCAs: roots, ServerName: "localhost", MinVersion: tls.VersionTLS12}
	if certificate != nil {
		config.Certificates = []tls.Certificate{*certificate}
	}
	rawConnection, err := (&net.Dialer{Timeout: 5 * time.Second}).DialContext(ctx, "tcp", address)
	if err != nil {
		t.Fatalf("dial %s: %v", address, err)
	}
	connection := tls.Client(rawConnection, config)
	if err := connection.HandshakeContext(ctx); err != nil {
		_ = rawConnection.Close()
		t.Fatalf("TLS handshake %s: %v", address, err)
	}
	logFactory := rpc.NewSimpleLogFactory(rpc.NilLogOutput{}, nil)
	transport := rpc.NewTransport(
		ctx,
		connection,
		logFactory,
		nil,
		rem.RegMakeGenericErrorWrapper(core.ErrorToStatus),
		core.RpcMaxSz,
	)
	return rpc.NewClient(transport, nil, nil), func() { _ = connection.Close() }
}

func liveCurrentRoot(
	t *testing.T,
	ctx context.Context,
	client *rem.MerkleQueryClient,
	host proto.HostID,
) proto.TreeRoot {
	t.Helper()
	signed, err := client.GetCurrentRootSigned(ctx, &host)
	if err != nil {
		t.Fatalf("get signed Merkle root: %v", err)
	}
	root, err := signed.Inner.AllocAndDecode(core.DecoderFactory{})
	if err != nil {
		t.Fatal(err)
	}
	var hash proto.MerkleRootHash
	if err := merkle.HashRoot(root, &hash); err != nil {
		t.Fatal(err)
	}
	return proto.TreeRoot{Epno: root.V1().Epno, Hash: hash}
}

func liveSignup(
	t *testing.T,
	ctx context.Context,
	reg *rem.RegClient,
	merkleClient *rem.MerkleQueryClient,
	host proto.HostID,
) goLiveUser {
	t.Helper()
	username := proto.Name("gocompat")
	if err := reg.CheckNameExists(ctx, username); err == nil {
		t.Fatal("checkNameExists accepted an unused name")
	}
	reservation, err := reg.ReserveUsername(ctx, username)
	if err != nil {
		t.Fatalf("reserve username: %v", err)
	}
	var deviceSeed, pukSeed proto.SecretSeed32
	for index := range deviceSeed {
		deviceSeed[index] = byte(index + 11)
		pukSeed[index] = byte(index + 81)
	}
	device, err := core.NewPrivateSuite25519(proto.EntityType_Device, proto.OwnerRole, deviceSeed, host)
	if err != nil {
		t.Fatal(err)
	}
	puk, err := core.NewSharedPrivateSuite25519(
		proto.EntityType_PUKVerify,
		proto.OwnerRole,
		pukSeed,
		proto.FirstGeneration,
		host,
	)
	if err != nil {
		t.Fatal(err)
	}
	label := proto.DeviceLabel{
		DeviceType: proto.DeviceType_Computer,
		Name:       proto.DeviceNameNormalized("go client"),
		Serial:     proto.FirstDeviceSerial,
	}
	eldest, err := core.MakeEldestLink(
		host,
		rem.NameCommitment{Name: username, Seq: reservation.Seq},
		device,
		puk,
		label,
		liveCurrentRoot(t, ctx, merkleClient, host),
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	devicePublic, err := device.Publicize(&host)
	if err != nil {
		t.Fatal(err)
	}
	pukBox, err := core.BoxOne(host, puk, device, devicePublic)
	if err != nil {
		t.Fatal(err)
	}
	pukEntity, err := puk.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	uidEntity, err := pukEntity.Persistent(proto.PartyType_User)
	if err != nil {
		t.Fatal(err)
	}
	uid, err := uidEntity.ToUID()
	if err != nil {
		t.Fatal(err)
	}
	selfToken, err := core.NewPermissionToken()
	if err != nil {
		t.Fatal(err)
	}
	argument := rem.SignupArg{
		UsernameUtf8:          proto.NameUtf8(username),
		Rur:                   reservation,
		Link:                  *eldest.Link,
		PukBox:                *pukBox,
		UsernameCommitmentKey: *eldest.UsernameCommitmentKey,
		Dlnck: rem.DeviceLabelNameAndCommitmentKey{
			Dln: proto.DeviceLabelAndName{
				Label: label,
				Nv:    proto.NormalizationVersion_V0,
				Name:  proto.DeviceName("go client"),
			},
			CommitmentKey: *eldest.DevNameCommitmentKey,
		},
		NextTreeLocation:         *eldest.NextTreeLocation,
		InviteCode:               rem.NewInviteCodeWithEmpty(),
		Email:                    proto.Email("gocompat@example.com"),
		SubchainTreeLocationSeed: *eldest.SubchainTreeLocationSeed,
		SelfToken:                selfToken,
		Hepks:                    *eldest.HEPKSet,
		Sso:                      rem.NewRegSSOArgsWithNone(),
	}
	if err := reg.Signup(ctx, argument); err != nil {
		t.Fatalf("official signup: %v", err)
	}
	deviceID, err := device.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	deviceIDFixed, err := deviceID.ToDeviceID()
	if err != nil {
		t.Fatal(err)
	}
	if err := reg.ProbeKeyExists(ctx, rem.ProbeKeyExistsArg{Uid: uid, DevID: deviceIDFixed, SelfTok: selfToken}); err != nil {
		t.Fatalf("activate provisional signup key: %v", err)
	}
	linkHash, err := core.LinkHash(eldest.Link)
	if err != nil {
		t.Fatal(err)
	}
	return goLiveUser{
		host: host, uid: uid, device: device, puk: puk, prev: *linkHash,
		nextSeqno: eldest.Seqno + 1,
	}
}

func liveProvision(
	t *testing.T,
	ctx context.Context,
	userClient *rem.UserClient,
	regClient *rem.RegClient,
	merkleClient *rem.MerkleQueryClient,
	user *goLiveUser,
) {
	t.Helper()
	var seed proto.SecretSeed32
	for index := range seed {
		seed[index] = byte(0xd0 + index)
	}
	newDevice, err := core.NewPrivateSuite25519(proto.EntityType_Device, proto.OwnerRole, seed, user.host)
	if err != nil {
		t.Fatal(err)
	}
	newHEPK, err := newDevice.ExportHEPK()
	if err != nil {
		t.Fatal(err)
	}
	existing, err := user.device.Publicize(&user.host)
	if err != nil {
		t.Fatal(err)
	}
	label := proto.DeviceLabel{
		DeviceType: proto.DeviceType_Computer,
		Name:       proto.DeviceNameNormalized("go provisioned"),
		Serial:     proto.FirstDeviceSerial,
	}
	link, err := core.MakeProvisionLink(
		user.uid,
		user.host,
		existing,
		newDevice,
		proto.OwnerRole,
		nil,
		label,
		user.nextSeqno,
		user.prev,
		liveCurrentRoot(t, ctx, merkleClient, user.host),
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	link.Link, err = core.CountersignProvisionLink(link.Link, user.device)
	if err != nil {
		t.Fatal(err)
	}
	boxer, err := core.NewSharedKeyBoxer(user.host, user.device)
	if err != nil {
		t.Fatal(err)
	}
	newPublic, err := newDevice.Publicize(&user.host)
	if err != nil {
		t.Fatal(err)
	}
	if err := boxer.Box(user.puk, newPublic); err != nil {
		t.Fatal(err)
	}
	boxes, err := boxer.Finish()
	if err != nil {
		t.Fatal(err)
	}
	selfToken, err := core.NewPermissionToken()
	if err != nil {
		t.Fatal(err)
	}
	if err := userClient.ProvisionDevice(ctx, rem.ProvisionDeviceArg{
		Link: *link.Link,
		Dlnc: rem.DeviceLabelNameAndCommitmentKey{
			Dln:           proto.DeviceLabelAndName{Label: label, Nv: proto.NormalizationVersion_V0, Name: "go provisioned"},
			CommitmentKey: *link.DevNameCommitmentKey,
		},
		PukBoxes:         *boxes,
		NextTreeLocation: *link.NextTreeLocation,
		SelfToken:        selfToken,
		Hepks:            proto.HEPKSet{V: []proto.HEPK{*newHEPK}},
	}); err != nil {
		t.Fatalf("official device provision: %v", err)
	}
	newID, err := newDevice.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	newDeviceID, err := newID.ToDeviceID()
	if err != nil {
		t.Fatal(err)
	}
	if err := regClient.ProbeKeyExists(ctx, rem.ProbeKeyExistsArg{Uid: user.uid, DevID: newDeviceID, SelfTok: selfToken}); err != nil {
		t.Fatalf("activate provisioned key: %v", err)
	}
	user.prev, err = func() (proto.LinkHash, error) {
		value, err := core.LinkHash(link.Link)
		if err != nil {
			return proto.LinkHash{}, err
		}
		return *value, nil
	}()
	if err != nil {
		t.Fatal(err)
	}
	user.nextSeqno++
}

func liveSetPassphrase(
	t *testing.T,
	ctx context.Context,
	userClient *rem.UserClient,
	merkleClient *rem.MerkleQueryClient,
	user *goLiveUser,
) {
	t.Helper()
	passphrase := proto.Passphrase("go compatibility passphrase")
	salt := core.RandomPassphraseSalt()
	stretchVersion, err := userClient.StretchVersion(ctx)
	if err != nil {
		t.Fatalf("official passphrase stretch version: %v", err)
	}
	stretched, err := libclient.NewStretchedPassphrase(
		libclient.StretchOpts{},
		passphrase,
		salt,
		proto.FirstPassphraseGeneration,
		stretchVersion,
	)
	if err != nil {
		t.Fatal(err)
	}
	var wrappingKey lcl.SKMWK
	var sessionKey lcl.PpeSessionKey
	if err := core.RandomFill(wrappingKey[:]); err != nil {
		t.Fatal(err)
	}
	if err := core.RandomFill(sessionKey[:]); err != nil {
		t.Fatal(err)
	}
	wrappedKeys, err := core.SealIntoSecretBox(
		&lcl.SKMWKList{Fqu: proto.FQUser{Uid: user.uid, HostID: user.host}, Keys: []lcl.SKMWK{wrappingKey}},
		(*proto.SecretBoxKey)(&sessionKey),
	)
	if err != nil {
		t.Fatal(err)
	}
	passphrasePublic, err := stretched.PublicKeySuite()
	if err != nil {
		t.Fatal(err)
	}
	passphraseBox, err := core.BoxForEmphemeral(
		&lcl.PpePassphraseBoxPayload{Gen: proto.FirstPassphraseGeneration, Sesskey: sessionKey},
		passphrasePublic,
		core.BoxOpts{IncludePublicKey: true},
	)
	if err != nil {
		t.Fatal(err)
	}
	hepk, err := passphrasePublic.ExportHEPK()
	if err != nil {
		t.Fatal(err)
	}
	pukBox, err := core.SealIntoSecretBox(
		&lcl.PpePUKBoxPayload{
			Gen: proto.FirstPassphraseGeneration, Sesskey: sessionKey, Passphrase: *hepk,
		},
		func() *proto.SecretBoxKey { key := user.puk.SecretBoxKey(); return &key }(),
	)
	if err != nil {
		t.Fatal(err)
	}
	settings, err := core.MakeGenericLink(
		user.uid.EntityID(),
		user.host,
		user.device,
		proto.NewGenericLinkPayloadWithUsersettings(
			proto.NewUserSettingsLinkWithPassphrase(proto.PassphraseInfo{
				Gen: proto.FirstPassphraseGeneration, Salt: &salt, Sv: stretchVersion,
			}),
		),
		proto.ChainEldestSeqno,
		nil,
		liveCurrentRoot(t, ctx, merkleClient, user.host),
	)
	if err != nil {
		t.Fatal(err)
	}
	argument := rem.SetPassphraseArg{
		StretchVersion: stretchVersion,
		Key:            passphrasePublic.GetEntityID(),
		Salt:           salt,
		SkwkBox:        *wrappedKeys,
		PukBox: &proto.PpePUKBox{
			Box: *pukBox, PukRole: proto.OwnerRole, PukGen: proto.FirstGeneration,
		},
		PassphraseBox: proto.PpePassphraseBox{Box: *passphraseBox},
		UserSettingsLink: &rem.PostGenericLinkArg{
			Link: *settings.Link, NextTreeLocation: *settings.NextTreeLocation,
		},
	}
	if err := userClient.SetPassphrase(ctx, argument); err != nil {
		t.Fatalf("official passphrase set: %v", err)
	}
	loaded, err := userClient.LoadGenericChain(ctx, rem.LoadGenericChainArg{
		Eid: user.uid.EntityID(), Typ: proto.ChainType_UserSettings, Start: proto.ChainEldestSeqno,
	})
	if err != nil || len(loaded.Links) != 1 {
		t.Fatalf("load Go settings chain: links=%d err=%v", len(loaded.Links), err)
	}
}

func liveCreateAndLoadTeam(
	t *testing.T,
	ctx context.Context,
	authRPC *rpc.Client,
	userClient *rem.UserClient,
	merkleClient *rem.MerkleQueryClient,
	user *goLiveUser,
) proto.TeamID {
	t.Helper()
	roles := teamlib.EldestRoles()
	keys := make([]core.SharedPrivateSuiter, 0, len(roles))
	hepks := proto.HEPKSet{}
	for roleIndex, role := range roles {
		var seed proto.SecretSeed32
		for index := range seed {
			seed[index] = byte(31*roleIndex + index + 7)
		}
		key, err := core.NewSharedPrivateSuite25519(
			proto.EntityType_NamedTeam, role, seed, proto.FirstGeneration, user.host,
		)
		if err != nil {
			t.Fatal(err)
		}
		hepk, err := key.ExportHEPK()
		if err != nil {
			t.Fatal(err)
		}
		keys = append(keys, key)
		hepks.V = append(hepks.V, *hepk)
	}
	ownerPublic, err := core.PublicizeToSPSBoxer(
		user.puk,
		proto.FQUser{Uid: user.uid, HostID: user.host}.FQParty(),
	)
	if err != nil {
		t.Fatal(err)
	}
	boxer, err := core.NewSharedKeyBoxer(user.host, user.puk)
	if err != nil {
		t.Fatal(err)
	}
	for _, key := range keys {
		if err := boxer.Box(key, ownerPublic); err != nil {
			t.Fatal(err)
		}
	}
	boxes, err := boxer.Finish()
	if err != nil {
		t.Fatal(err)
	}
	root := liveCurrentRoot(t, ctx, merkleClient, user.host)
	owner := proto.KeyOwner{Party: user.uid.ToPartyID(), SrcRole: proto.OwnerRole}
	eldest, err := teamlib.MakeEldestLink(user.host, nil, owner, user.puk, keys, root, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	teamID, err := eldest.TeamID.ToTeamID()
	if err != nil {
		t.Fatal(err)
	}
	membership, err := core.MakeGenericLink(
		user.uid.EntityID(),
		user.host,
		user.device,
		proto.NewGenericLinkPayloadWithTeammembership(proto.TeamMembershipLink{
			Team:    proto.FQTeam{Team: teamID, Host: user.host},
			SrcRole: proto.OwnerRole,
			State: proto.NewTeamMembershipDetailsWithApprovedadhoc(proto.RoleAndSeqno{
				Role: proto.OwnerRole, Seqno: proto.ChainEldestSeqno,
			}),
		}),
		proto.ChainEldestSeqno,
		nil,
		root,
	)
	if err != nil {
		t.Fatal(err)
	}
	teamAdmin := rem.TeamAdminClient{Cli: authRPC, ErrorUnwrapper: core.StatusToError}
	if err := teamAdmin.CreateTeamAdHoc(ctx, rem.CreateTeamCommonArg{
		SubchainTreeLocationSeed: *eldest.SubchainTreeLocationSeed,
		Eta: rem.EditTeamArg{
			Link: *eldest.Link, NextTreeLocation: *eldest.NextTreeLocation,
			Obd: rem.OffchainBoxData{PtkBoxes: *boxes, Hepks: hepks},
		},
		TeamMembershipLink: rem.PostGenericLinkArg{
			Link: *membership.Link, NextTreeLocation: *membership.NextTreeLocation,
		},
	}); err != nil {
		t.Fatalf("official ad-hoc team create: %v", err)
	}
	teamList, err := userClient.GetTeamListServerTrust(ctx)
	if err != nil || len(teamList) != 1 || !teamList[0].Id.Eq(teamID) {
		t.Fatalf("trusted Go team list: list=%v err=%v", teamList, err)
	}
	loader := rem.TeamLoaderClient{Cli: authRPC, ErrorUnwrapper: core.StatusToError}
	request := rem.TeamVOBearerTokenReq{
		Team: proto.FQTeamIDOrName{
			Host: user.host, IdOrName: proto.NewTeamIDOrNameWithTrue(teamID.EntityID()),
		},
		Member:  proto.FQParty{Party: user.uid.ToPartyID(), Host: user.host},
		SrcRole: proto.OwnerRole,
		Gen:     proto.FirstGeneration,
	}
	challenge, err := loader.GetTeamVOBearerTokenChallenge(ctx, request)
	if err != nil {
		t.Fatalf("team view challenge: %v", err)
	}
	signature, err := user.puk.Sign(&challenge)
	if err != nil {
		t.Fatal(err)
	}
	token, err := loader.ActivateTeamVOBearerToken(ctx, rem.ActivateTeamVOBearerTokenArg{
		Ch: challenge, Sig: *signature,
	})
	if err != nil {
		t.Fatalf("team view activation: %v", err)
	}
	rosterUser, err := userClient.LoadUserChain(ctx, rem.LoadUserChainArg{
		Uid: user.uid, Start: proto.ChainEldestSeqno,
		Auth: rem.NewLoadUserChainAuthWithAslocalteam(token.Tok),
	})
	if err != nil || len(rosterUser.Links) != 2 {
		t.Fatalf("team-authorized roster user load: links=%d err=%v", len(rosterUser.Links), err)
	}
	loaded, err := loader.LoadTeamChain(ctx, rem.LoadTeamChainArg{
		Team:  proto.FQTeam{Team: teamID, Host: user.host},
		Tok:   rem.NewTokenVariantWithTeamvobearer(token.Tok),
		Start: proto.ChainEldestSeqno,
	})
	if err != nil || len(loaded.Links) != 1 {
		t.Fatalf("official team load: links=%d err=%v", len(loaded.Links), err)
	}
	return teamID
}

func liveKVPutGet(
	t *testing.T,
	ctx context.Context,
	authRPC *rpc.Client,
	user *goLiveUser,
) {
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
}
