package main

import (
	"errors"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/team"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
)

func ownerGuardMember(u byte) team.MemberID {
	var fqe proto.FQEntityFixed
	fqe.Host[0] = byte(proto.EntityType_Host)
	fqe.Host[32] = 1
	fqe.Entity[0] = byte(proto.EntityType_User)
	fqe.Entity[33] = u
	return team.MemberID{Fqe: fqe, SrcRole: core.OwnerRole}
}

// TestTeamOwnerGuard pins the go-foks main roster rule (df2acf1) that the
// Rust client and server reproduce: with RequireOwner, a change set that
// takes the roster from at least one owner to none is a TEAM_ROSTER_ERROR;
// without it (chain replay), the same change set is accepted.
func TestTeamOwnerGuard(t *testing.T) {
	owner := core.RoleKey{Typ: proto.RoleType_OWNER}
	reader := core.RoleKey{Typ: proto.RoleType_MEMBER, Lev: proto.VizLevel(0)}
	kvMin := core.KVMinRole
	alice := ownerGuardMember(1)
	bob := ownerGuardMember(2)

	roster := team.NewRosterCore()
	roster.Add(alice, owner, 1, 1, 0)
	roster.Add(bob, reader, 1, 2, 0)
	keys := team.NewKeyGens()
	keys[owner] = 1
	keys[core.AdminRole] = 1
	keys[reader] = 1
	keys[kvMin] = 1

	// Sole-owner self-demotion to the lowest member role.
	demote := team.ChangeSet{{Member: alice, Info: team.MemberInfo{Role: kvMin, Gen: 1}}}

	_, _, _, err := demote.Gameplan(alice, roster, keys, &team.GameplanOpts{RequireOwner: true})
	var rosterErr core.TeamRosterError
	if !errors.As(err, &rosterErr) || string(rosterErr) != "change would leave team without an owner" {
		t.Fatalf("RequireOwner result: %v", err)
	}
	if sc := core.ErrorToStatus(err).Sc; sc != proto.StatusCode_TEAM_ROSTER_ERROR {
		t.Fatalf("status %v", sc)
	}

	post, _, _, err := demote.Gameplan(alice, roster, keys, nil)
	if err != nil {
		t.Fatalf("replay without RequireOwner: %v", err)
	}
	if post.HasOwner() {
		t.Fatal("replayed demotion should leave the roster without an owner")
	}

	// Promoting bob in the same change set keeps an owner.
	handoff := team.ChangeSet{
		{Member: alice, Info: team.MemberInfo{Role: kvMin, Gen: 1}},
		{Member: bob, Info: team.MemberInfo{Role: owner, Gen: 1}},
	}
	if _, _, _, err := handoff.Gameplan(alice, roster, keys, &team.GameplanOpts{RequireOwner: true}); err != nil {
		t.Fatalf("handoff: %v", err)
	}
}

// TestTeamStatusFixtures emits the status frames a go-foks main server sends
// for an unknown view-only bearer token (5a9ff13) and for an edit that would
// leave a team without an owner (df2acf1).
func TestTeamStatusFixtures(t *testing.T) {
	d := newFixtureDir(t, "team")
	const sequence = rpc.SeqNumber(1)

	notFound, err := errorResponseFrame(sequence, core.ErrorToStatus(core.TeamVOBearerTokenNotFoundError{}))
	if err != nil {
		t.Fatal(err)
	}
	direct, err := errorResponseFrame(sequence, proto.NewStatusWithTeamVoBearerTokenNotFoundError())
	if err != nil {
		t.Fatal(err)
	}
	if string(notFound) != string(direct) {
		t.Fatal("ErrorToStatus and the generated constructor disagree")
	}
	d.put("vo-bearer-token-not-found-response.frame", notFound)

	ownerless, err := errorResponseFrame(sequence, core.ErrorToStatus(core.TeamRosterError("edit would leave team without an owner")))
	if err != nil {
		t.Fatal(err)
	}
	d.put("ownerless-edit-roster-error-response.frame", ownerless)
	d.finish()
}
