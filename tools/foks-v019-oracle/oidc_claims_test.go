package main

import (
	"bytes"
	"crypto/rand"
	"crypto/rsa"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"github.com/golang-jwt/jwt/v5"
	"math/big"
	"os"
	"path/filepath"
	"testing"
)

// A deterministic IdP fixture signs positive and negative claims with a test-only key.
func TestOIDCClaimFixtures(t *testing.T) {
	root := os.Getenv("FOKS_OIDC_FIXTURE_OUT")
	generate := root != ""
	if !generate {
		root = filepath.Join("..", "..", "crates", "foks-oidc", "tests", "fixtures")
	}
	if generate {
		if e := os.MkdirAll(root, 0755); e != nil {
			t.Fatal(e)
		}
	}
	keyPath := filepath.Join(root, "TEST_ONLY_RSA_KEY.pem")
	b, e := os.ReadFile(keyPath)
	if os.IsNotExist(e) && generate {
		key, err := rsa.GenerateKey(rand.Reader, 2048)
		if err != nil {
			t.Fatal(err)
		}
		b = pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)})
		if e = os.WriteFile(keyPath, b, 0600); e != nil {
			t.Fatal(e)
		}
	} else if e != nil {
		t.Fatal(e)
	}
	block, _ := pem.Decode(b)
	if block == nil {
		t.Fatal("missing test RSA key")
	}
	key, e := x509.ParsePKCS1PrivateKey(block.Bytes)
	if e != nil {
		t.Fatal(e)
	}
	put := func(name string, b []byte) {
		t.Helper()
		path := filepath.Join(root, name)
		if generate {
			if e := os.WriteFile(path, b, 0644); e != nil {
				t.Fatal(e)
			}
		} else {
			old, e := os.ReadFile(path)
			if e != nil {
				t.Fatal(e)
			}
			if !bytes.Equal(old, b) {
				t.Fatalf("fixture changed: %s", name)
			}
		}
	}
	jwks := map[string]any{"keys": []any{map[string]any{"kty": "RSA", "alg": "RS256", "use": "sig", "kid": "fixture", "n": base64.RawURLEncoding.EncodeToString(key.N.Bytes()), "e": base64.RawURLEncoding.EncodeToString(big.NewInt(int64(key.E)).Bytes())}}}
	b, e = json.Marshal(jwks)
	if e != nil {
		t.Fatal(e)
	}
	put("jwks.json", b)
	cases := map[string]func(jwt.MapClaims){
		"valid":                    func(c jwt.MapClaims) {},
		"wrong-issuer":             func(c jwt.MapClaims) { c["iss"] = "https://other.example" },
		"issuer-slash":             func(c jwt.MapClaims) { c["iss"] = "https://idp.example/" },
		"wrong-audience":           func(c jwt.MapClaims) { c["aud"] = "other" },
		"wrong-nonce":              func(c jwt.MapClaims) { c["nonce"] = "other" },
		"expired":                  func(c jwt.MapClaims) { c["exp"] = 1700000000 },
		"missing-subject":          func(c jwt.MapClaims) { delete(c, "sub") },
		"empty-subject":            func(c jwt.MapClaims) { c["sub"] = "" },
		"missing-expiry":           func(c jwt.MapClaims) { delete(c, "exp") },
		"wrong-azp":                func(c jwt.MapClaims) { c["azp"] = "other" },
		"multiple-audience-no-azp": func(c jwt.MapClaims) { c["aud"] = []string{"foks", "other"} },
		"multiple-audience":        func(c jwt.MapClaims) { c["aud"] = []string{"foks", "other"}; c["azp"] = "foks" },
		"future-issued":            func(c jwt.MapClaims) { c["iat"] = 1700000200 },
		"claim-type":               func(c jwt.MapClaims) { c["sub"] = 42 },
		"missing-nonce":            func(c jwt.MapClaims) { delete(c, "nonce") },
		"wrong-algorithm":          func(c jwt.MapClaims) {},
		"wrong-key-id":             func(c jwt.MapClaims) {},
	}
	for name, change := range cases {
		claims := jwt.MapClaims{"iss": "https://idp.example", "sub": "stable-subject", "aud": "foks", "nonce": "fixture-nonce", "iat": 1700000000, "exp": 1700000600, "preferred_username": "alice", "email": "alice@example.com"}
		change(claims)
		token := jwt.NewWithClaims(jwt.SigningMethodRS256, claims)
		token.Header["kid"] = "fixture"
		if name == "wrong-key-id" {
			token.Header["kid"] = "other"
		}
		var signed string
		var e error
		if name == "wrong-algorithm" {
			token.Method = jwt.SigningMethodHS256
			signed, e = token.SignedString([]byte("test-only-hmac-key"))
		} else {
			signed, e = token.SignedString(key)
		}
		if e != nil {
			t.Fatal(e)
		}
		put(name+".jwt", []byte(signed))
	}
}
