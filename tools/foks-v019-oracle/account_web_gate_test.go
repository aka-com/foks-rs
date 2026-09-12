package main

import (
	"github.com/foks-proj/go-foks/integration-tests/common"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/server/shared"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"testing"
)

// Test-only control for expiring sessions in the disposable upstream database.
// No hosted service or browser is contacted by this gate.
func accountAdminGate(t *testing.T, environment *common.TestEnv, command *exec.Cmd) {
	t.Helper()
	m := environment.MetaContext()
	destination, err := shared.AdminBaseURL(m)
	if err != nil {
		t.Fatal(err)
	}
	// The Rust driver selects the primary service by the probe's 127.0.0.1
	// address. Go reflects that vhost selection in the generated admin origin.
	parsed, err := url.Parse(string(destination))
	if err != nil {
		t.Fatal(err)
	}
	parsed.Host = "127.0.0.1:" + parsed.Port()
	destination = proto.URLString(parsed.String())
	control := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/expire" {
			http.NotFound(w, r)
			return
		}
		db, err := m.Db(shared.DbTypeUsers)
		if err != nil {
			http.Error(w, "database", 500)
			return
		}
		defer db.Release()
		_, err = db.Exec(r.Context(), "UPDATE user_web_sessions SET etime=NOW()-interval '1 second'")
		if err != nil {
			http.Error(w, "expiry", 500)
			return
		}
		w.WriteHeader(http.StatusNoContent)
	}))
	t.Cleanup(control.Close)
	command.Env = append(os.Environ(), "FOKS_TEST_ADMIN_DESTINATION="+string(destination), "FOKS_TEST_ADMIN_CONTROL="+control.URL)
}
