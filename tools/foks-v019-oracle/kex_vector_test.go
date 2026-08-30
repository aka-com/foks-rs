package main

import (
	"bytes"
	"encoding/hex"
	"reflect"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/proto/lcl"
	proto "github.com/foks-proj/go-foks/proto/lib"
)

func TestKexInteropVector(t *testing.T) {
	secret := proto.KexSecret{1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1}
	hesp, err := core.KexSecretToHESP(secret)
	if err != nil {
		t.Fatal(err)
	}
	var key proto.HMACKey
	copy(key[:], secret[:])
	session, err := core.Hmac(
		ptr(lcl.NewKexKeyDerivationDefault(lcl.KexDerivationType_SessionID)),
		&key,
	)
	if err != nil {
		t.Fatal(err)
	}
	boxKey, err := core.Hmac(
		ptr(lcl.NewKexKeyDerivationDefault(lcl.KexDerivationType_SecretBoxKey)),
		&key,
	)
	if err != nil {
		t.Fatal(err)
	}
	wantHESP := proto.KexHESP{
		"cage", "32", "advice", "4", "letter", "128", "avoid",
		"16", "acoustic", "2", "doctor", "64", "amount",
	}
	if !reflect.DeepEqual(hesp, wantHESP) {
		t.Fatalf("HESP = %#v, want %#v", hesp, wantHESP)
	}
	if got := hex.EncodeToString(session[:]); got != "7efbeaaecd9291ea952864fe363ee175de54b420212ceacc444421cd19dd777d" {
		t.Fatalf("session key = %s", got)
	}
	if got := hex.EncodeToString(boxKey[:]); got != "e9c6819da5d9c7333a282f64c5c8da3ca6351259dcf9c8d81f90c87fc258fc4f" {
		t.Fatalf("secretbox key = %s", got)
	}
	sender, err := proto.EntityType_Device.MakeEntityID(bytes.Repeat([]byte{2}, 32))
	if err != nil {
		t.Fatal(err)
	}
	cleartext := lcl.KexCleartext{
		SeesionID: proto.KexSessionID(*session),
		Sender:    sender,
		Seq:       0,
		Msg:       lcl.NewKexMsgWithStart(),
	}
	encoded, err := core.EncodeToBytes(&cleartext)
	if err != nil {
		t.Fatal(err)
	}
	var nonce proto.NaclNonce
	for index := range nonce {
		nonce[index] = 3
	}
	var secretboxKey proto.SecretBoxKey
	copy(secretboxKey[:], boxKey[:])
	ciphertext, err := core.SealIntoSecretBoxWithNonce(&cleartext, &nonce, &secretboxKey)
	if err != nil {
		t.Fatal(err)
	}
	if got := hex.EncodeToString(encoded); got != "94c4207efbeaaecd9291ea952864fe363ee175de54b420212ceacc444421cd19dd777dc42104020202020202020202020202020202020202020202020202020202020202020200920180" {
		t.Fatalf("cleartext = %s", got)
	}
	if got := hex.EncodeToString(ciphertext); got != "268c7c445e35566466e0d88d08b32926a7abb4d4566803cb676529d819abc21d9bbb9e346b75d253aed9eaee7c9120b0714c2d4812ae9b67a161f936fe03a7fa31775f4d17da90a3fbf92e5c9b808f27384de7558b6046156ac1" {
		t.Fatalf("ciphertext = %s", got)
	}
}

func ptr[T any](value T) *T { return &value }
