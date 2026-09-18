package launch

import "testing"

func TestQuote(t *testing.T) {
	got := Quote([]string{"hunk", "diff", "a b", "it's", ""})
	want := `hunk diff 'a b' 'it'\''s' ''`
	if got != want {
		t.Fatalf("Quote = %s, want %s", got, want)
	}
}
