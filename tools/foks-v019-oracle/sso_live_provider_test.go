package main

import (
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"github.com/foks-proj/go-foks/integration-tests/common"
	p "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/server/shared"
	"github.com/golang-jwt/jwt/v5"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

type strictOIDCCode struct{ nonce, challenge, redirect, username string }
type strictOIDC struct {
	server   *httptest.Server
	key      *rsa.PrivateKey
	jwks     []byte
	mu       sync.Mutex
	codes    map[string]strictOIDCCode
	refresh  map[string]string
	next     int
	username string
	callback string
}

func newStrictOIDC(t *testing.T) *strictOIDC {
	t.Helper()
	root := filepath.Join("..", "..", "crates", "foks-oidc", "tests", "fixtures")
	b, e := os.ReadFile(filepath.Join(root, "TEST_ONLY_RSA_KEY.pem"))
	if e != nil {
		t.Fatal(e)
	}
	block, _ := pem.Decode(b)
	key, e := x509.ParsePKCS1PrivateKey(block.Bytes)
	if e != nil {
		t.Fatal(e)
	}
	jwks, e := os.ReadFile(filepath.Join(root, "jwks.json"))
	if e != nil {
		t.Fatal(e)
	}
	s := &strictOIDC{key: key, jwks: jwks, codes: make(map[string]strictOIDCCode), refresh: make(map[string]string), username: "rustsso"}
	s.server = httptest.NewServer(http.HandlerFunc(s.serve))
	t.Cleanup(s.server.Close)
	return s
}
func (s *strictOIDC) serve(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	switch r.URL.Path {
	case "/discovery":
		_ = json.NewEncoder(w).Encode(map[string]any{"issuer": s.server.URL, "authorization_endpoint": s.server.URL + "/authorize", "token_endpoint": s.server.URL + "/token", "jwks_uri": s.server.URL + "/jwks", "userinfo_endpoint": s.server.URL + "/userinfo", "response_types_supported": []string{"code"}, "subject_types_supported": []string{"public"}, "id_token_signing_alg_values_supported": []string{"RS256"}, "scopes_supported": []string{"openid", "email", "profile", "offline_access"}})
	case "/jwks":
		_, _ = w.Write(s.jwks)
	case "/authorize":
		q := r.URL.Query()
		if q.Get("client_id") != "fennec" || q.Get("response_type") != "code" || q.Get("code_challenge_method") != "S256" || len(q.Get("code_challenge")) != 43 || q.Get("nonce") == "" || q.Get("state") == "" {
			http.Error(w, "invalid authorization", 400)
			return
		}
		redirect, e := url.Parse(q.Get("redirect_uri"))
		if e != nil || (redirect.Scheme != "https" && !(redirect.Scheme == "http" && (redirect.Hostname() == "localhost" || redirect.Hostname() == "127.0.0.1"))) {
			http.Error(w, "invalid callback", 400)
			return
		}
		s.mu.Lock()
		if redirect.String() != s.callback {
			s.mu.Unlock()
			http.Error(w, "callback substitution: expected "+s.callback+" got "+redirect.String(), 400)
			return
		}
		s.next++
		code := fmt.Sprintf("one-use-%d", s.next)
		s.codes[code] = strictOIDCCode{q.Get("nonce"), q.Get("code_challenge"), redirect.String(), s.username}
		s.mu.Unlock()
		out := redirect.Query()
		out.Set("code", code)
		out.Set("state", q.Get("state"))
		redirect.RawQuery = out.Encode()
		http.Redirect(w, r, redirect.String(), http.StatusFound)
	case "/token":
		if r.Method != "POST" || r.ParseForm() != nil || r.Form.Get("client_id") != "fennec" {
			http.Error(w, "invalid token request", 400)
			return
		}
		s.mu.Lock()
		defer s.mu.Unlock()
		var nonce, username string
		switch r.Form.Get("grant_type") {
		case "authorization_code":
			code, ok := s.codes[r.Form.Get("code")]
			if !ok {
				http.Error(w, "replayed code", 400)
				return
			}
			delete(s.codes, r.Form.Get("code"))
			verifier := r.Form.Get("code_verifier")
			sum := sha256.Sum256([]byte(verifier))
			if len(verifier) < 43 || len(verifier) > 128 || base64.RawURLEncoding.EncodeToString(sum[:]) != code.challenge || r.Form.Get("redirect_uri") != code.redirect {
				http.Error(w, "invalid PKCE or redirect", 400)
				return
			}
			nonce = code.nonce
			username = code.username
		case "refresh_token":
			old := r.Form.Get("refresh_token")
			if s.refresh[old] == "" {
				w.WriteHeader(400)
				_, _ = w.Write([]byte(`{"error":"invalid_grant"}`))
				return
			}
			username = s.refresh[old]
			delete(s.refresh, old)
		default:
			http.Error(w, "invalid grant", 400)
			return
		}
		s.next++
		refresh := fmt.Sprintf("rotated-%d", s.next)
		s.refresh[refresh] = username
		now := time.Now().Unix()
		claims := jwt.MapClaims{"iss": s.server.URL, "sub": username + "-subject", "aud": "fennec", "iat": now, "exp": now + 600, "preferred_username": username, "email": username + "@example.test"}
		if nonce != "" {
			claims["nonce"] = nonce
		}
		tok := jwt.NewWithClaims(jwt.SigningMethodRS256, claims)
		tok.Header["kid"] = "fixture"
		signed, e := tok.SignedString(s.key)
		if e != nil {
			http.Error(w, "signing failed", 500)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"access_token": "opaque-access", "token_type": "Bearer", "refresh_token": refresh, "expires_in": 300, "id_token": signed})
	case "/userinfo":
		_ = json.NewEncoder(w).Encode(map[string]any{"sub": "strict-stable-subject", "email": "rustsso@example.test"})
	default:
		http.NotFound(w, r)
	}
}

func configureRustLiveSSO(t *testing.T, env *common.TestEnv, dir string) (func(), <-chan error) {
	t.Helper()
	idp := newStrictOIDC(t)
	callback, e := shared.OAuth2CallbackURL(env.MetaContext())
	if e != nil {
		t.Fatal(e)
	}
	// Primary-host Go callbacks use the selected probe lookup name. Register
	// that exact loopback URI, preserving the configured web port and path.
	cb, e := url.Parse(callback.String())
	if e != nil {
		t.Fatal(e)
	}
	lookup, _, e := net.SplitHostPort(env.ProbeSrv().ListenerAddr().String())
	if e != nil {
		t.Fatal(e)
	}
	cb.Host = net.JoinHostPort(lookup, cb.Port())
	idp.callback = cb.String()
	err := shared.SetVHostSSOConfig(env.MetaContext(), &p.SSOConfig{Active: p.SSOProtocolType_Oauth2, Oauth2: &p.OAuth2Config{ConfigURI: p.URLString(idp.server.URL + "/discovery"), ClientID: "fennec"}})
	if err != nil {
		t.Fatal(err)
	}
	stop := make(chan struct{})
	done := make(chan error, 1)
	go func() {
		var result error
		defer func() { done <- result }()
		next := 1
		client := env.HttpClient(15 * time.Second)
		for {
			select {
			case <-stop:
				return
			case <-time.After(20 * time.Millisecond):
			}
			path := filepath.Join(dir, fmt.Sprintf("sso-browser-%d.url", next))
			b, e := os.ReadFile(path)
			if os.IsNotExist(e) {
				continue
			}
			if e != nil {
				result = e
				return
			}
			if next == 3 {
				idp.mu.Lock()
				idp.username = "rustssoyubi"
				idp.mu.Unlock()
			}
			target := strings.TrimSpace(string(b))
			response, e := client.Get(target)
			if e != nil {
				result = fmt.Errorf("SSO browser request failed: %T", e)
				return
			}
			body, _ := io.ReadAll(io.LimitReader(response.Body, 4096))
			_ = response.Body.Close()
			if response.StatusCode != 200 {
				result = fmt.Errorf("SSO browser status %d: %s", response.StatusCode, body)
				return
			}
			if e = os.WriteFile(filepath.Join(dir, fmt.Sprintf("sso-browser-%d.done", next)), []byte("ok"), 0600); e != nil {
				result = e
				return
			}
			next++
		}
	}()
	return func() { close(stop) }, done
}
