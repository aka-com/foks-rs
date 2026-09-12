package main

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/foks-proj/go-foks/server/shared"
)

// Seed stale membership rows directly, then exercise the unmodified Go handler
// through the Rust client. No synthetic RPC response stands in for filtering.
func seedFilteredInbox(m shared.MetaContext, directory string, stop <-chan struct{}) error {
	for _, stage := range []struct {
		name  string
		count int
	}{{"persistent", 1000}, {"recoverable", 100}} {
		deadline := time.After(30 * time.Second)
		ticker := time.NewTicker(10 * time.Millisecond)
		ready := false
		for !ready {
			select {
			case <-stop:
				ticker.Stop()
				return nil
			case <-deadline:
				ticker.Stop()
				return fmt.Errorf("waiting for %s inbox seed request", stage.name)
			case <-ticker.C:
				_, err := os.Stat(filepath.Join(directory, stage.name+".request"))
				ready = err == nil
			}
		}
		ticker.Stop()
		db, err := m.Db(shared.DbTypeRealTime)
		if err != nil {
			return err
		}
		// The driver creates exactly one real channel. Reserved tiny IDs are
		// test-only stale channels with a valid but unowned parent team ID.
		queries := []string{
			`DELETE FROM user_channels WHERE channel_id BETWEEN 1 AND 1000`,
			`DELETE FROM channels WHERE channel_id BETWEEN 1 AND 1000`,
			fmt.Sprintf(`UPDATE user_channels SET inbox_version = inbox_version + %d`, stage.count+1),
			fmt.Sprintf(`INSERT INTO channels (
                short_host_id, channel_id, parent_team_id, app_id, channel_id_full,
                seqno, name_box, name_box_ptk_gen, tier, desc_box, desc_box_ptk_gen,
                read_role_type, read_role_viz_level, write_role_type, write_role_viz_level,
                ctime, mtime, updated_at_set_vers)
              SELECT c.short_host_id, i, set_byte(c.parent_team_id, 1, get_byte(c.parent_team_id, 1) # 128),
                c.app_id, decode(lpad(to_hex(i), 32, '0'), 'hex'),
                c.seqno, c.name_box, c.name_box_ptk_gen, c.tier, c.desc_box, c.desc_box_ptk_gen,
                c.read_role_type, c.read_role_viz_level, c.write_role_type, c.write_role_viz_level,
                c.ctime, c.mtime, c.updated_at_set_vers
              FROM channels c CROSS JOIN generate_series(1, %d) i
              WHERE c.channel_id > 1000`, stage.count),
			fmt.Sprintf(`INSERT INTO user_channels (
                short_host_id, channel_id, uid, app_id, inbox_version,
                last_msg_time, read_through, hidden, muted, ctime, mtime)
              SELECT short_host_id, i, uid, app_id, inbox_version - %d + i,
                last_msg_time, 0, false, false, ctime, mtime
              FROM user_channels CROSS JOIN generate_series(1, %d) i
              WHERE channel_id > 1000`, stage.count+1, stage.count),
			`UPDATE user_inbox SET inbox_version = (SELECT MAX(inbox_version) FROM user_channels)`,
		}
		for _, query := range queries {
			if _, err = db.Exec(m.Ctx(), query); err != nil {
				break
			}
		}
		db.Release()
		if err != nil {
			return fmt.Errorf("seed %s inbox: %w", stage.name, err)
		}
		if err := os.WriteFile(filepath.Join(directory, stage.name+".ready"), nil, 0600); err != nil {
			return err
		}
	}
	return nil
}
