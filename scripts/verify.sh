#!/usr/bin/env bash
# Verification gate of the repository, run before every commit: one line PASS, FAIL or NOT RUN per
# check, then the verdict. Exit code 0 when every check that could run passed, 1 when one failed, 2
# when the gate itself could not run.
#
#   bash scripts/verify.sh                 every check: searches, JavaScript, build, clippy, tests, audit
#   bash scripts/verify.sh --search-only   the searches only, in a second
#
# Every search runs beside a positive witness, a pattern that must be found in the same files: a
# search that finds nothing proves nothing until it is shown to read them. The output of cargo goes
# to target/verify/<check>.log. Runs with bash 3.2 (macOS) and later, and needs git and perl.
set -u
root=$(cd "$(dirname "$0")/.." && pwd) || exit 2
cd "$root" || exit 2

search_only=0
case "${1:-}" in
	"") ;;
	--search-only) search_only=1 ;;
	*)
		echo "usage: bash scripts/verify.sh [--search-only]"
		exit 2
		;;
esac

if ! command -v git >/dev/null || ! command -v perl >/dev/null; then
	echo "NOT RUN  git and perl are needed"
	exit 2
fi

failed=0
line() { printf '%-8s %-22s %s\n' "$1" "$2" "$3"; }
pass() { line PASS "$1" "$2"; }
fail() { line FAIL "$1" "$2"; failed=1; }
skip() { line "NOT RUN" "$1" "$2"; }

# The files searched: every text file of the repository, tracked or new and not ignored. The word
# lists of data/ are left out: they are data, not text of the project.
files=()
while IFS= read -r file; do
	files+=("$file")
done < <(git ls-files --cached --others --exclude-standard -- . ':!data' |
	grep -E '(\.rs|\.toml|\.md|\.html|\.css|\.js|\.yml|\.sh|\.svg|\.gitignore|LICENSE)$')
if [ "${#files[@]}" -eq 0 ]; then
	echo "NOT RUN  no file to search"
	exit 2
fi

# The files are read as UTF-8 characters (-CSD); a pattern passed through the environment arrives as
# bytes, so it is decoded first, or a pattern with an accent would never match.
# count <perl regex>: the number of lines of the searched files that match it.
count() {
	RX="$1" perl -CSD -MEncode -ne 'BEGIN { my $s = Encode::decode("UTF-8", $ENV{RX}); $r = qr/$s/ } $n++ if /$r/; END { print $n + 0 }' "${files[@]}"
}
# where <perl regex>: the first places that match it, file and line.
where() {
	RX="$1" perl -CSD -MEncode -ne 'BEGIN { my $s = Encode::decode("UTF-8", $ENV{RX}); $r = qr/$s/ } print "$ARGV:$. " if /$r/; close ARGV if eof' "${files[@]}" | cut -c1-120
}
# search <name> <regex> <witness regex>: no line may match the regex, and the witness must be found.
search() {
	local found witness
	found=$(count "$2")
	witness=$(count "$3")
	if [ "$witness" -eq 0 ]; then
		fail "$1" "witness not found: the search reads nothing"
	elif [ "$found" -ne 0 ]; then
		fail "$1" "$found line(s): $(where "$2")"
	else
		pass "$1" "0 line, witness $witness"
	fi
}

# 1. No em dash nor en dash, anywhere. The witness, an accented letter, shows that the files are
#    read as UTF-8.
search "dashes" '[\x{2013}\x{2014}]' 'é'
# 2. No invisible character that would make a text read differently from what it says.
search "invisible characters" '[\x{200B}-\x{200D}\x{202A}-\x{202E}\x{2066}-\x{2069}\x{FEFF}]' 'é'
# 3. No secret: private keys and the tokens of the usual services. The witness is this script.
search "secrets" '-----BEGIN [A-Z ]*PRIVATE KEY-----|ghp_[A-Za-z0-9]{20,}|glpat-[A-Za-z0-9_-]{20,}|sk-ant-[A-Za-z0-9_-]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}' 'glpat-'
# 4. No path of a machine: a path is relative to the repository.
search "machine paths" '/Users/[A-Za-z]|[A-Za-z]:\\Users\\[A-Za-z]|/home/[a-z][a-z0-9_-]+/' '\.\./'
# 5. No mention of the tools that helped write the code. The brackets keep this line out of the
#    search; the witness is the name of the author.
search "tool mentions" '(?i)\bc[l]aude\b|co-a[u]thored-by|a[n]thropic' 'jferdelyi'
# 6. LF line endings only.
search "line endings" '\r$' '.'

# 7. The scripts of the web front parse. It has no build step: a syntax error would only show in
#    the browser.
if [ "$search_only" -eq 1 ]; then
	skip "javascript" "--search-only"
elif ! command -v node >/dev/null; then
	skip "javascript" "node is not installed"
else
	bad=""
	for script in simple-web-front/*.js; do
		node --check "$script" 2>/dev/null || bad="$bad $script"
	done
	if [ -n "$bad" ]; then
		fail "javascript" "node --check refuses:$bad"
	else
		pass "javascript" "node --check on $(ls simple-web-front/*.js | wc -l | tr -d ' ') script(s)"
	fi
fi

# 8 to 10. The same steps as the CI, with the versions of Cargo.lock. Warnings are denied in
#    Cargo.toml, so a warning fails the build or clippy.
logs=target/verify
cargo_check() {
	local name="$1"
	shift
	if [ "$search_only" -eq 1 ]; then
		skip "$name" "--search-only"
	elif ! command -v cargo >/dev/null; then
		skip "$name" "cargo is not installed"
	elif cargo "$@" >"$logs/$name.log" 2>&1; then
		pass "$name" "cargo $*"
	else
		fail "$name" "cargo $*, see $logs/$name.log"
	fi
}
mkdir -p "$logs" || exit 2
cargo_check "build" build --locked --all-targets
cargo_check "clippy" clippy --locked --all-targets
cargo_check "tests" test --locked

# 11. No locked dependency with a known vulnerability. cargo audit fetches the RustSec advisory
#     database at each run: a new advisory fails the gate without any change to the code. Warnings
#     (unmaintained or unsound crates) are counted, not denied. One advisory is ignored:
#     RUSTSEC-2026-0258, h2 0.3 with no fix in that branch. The server binds with
#     HttpServer::bind, which serves HTTP/1.x only, so the HTTP/2 code of h2 is never reached.
audit_ignored="RUSTSEC-2026-0258"
if [ "$search_only" -eq 1 ]; then
	skip "audit" "--search-only"
elif ! command -v cargo >/dev/null; then
	skip "audit" "cargo is not installed"
elif ! command -v cargo-audit >/dev/null; then
	skip "audit" "cargo-audit is not installed"
else
	cargo audit --json --ignore "$audit_ignored" >"$logs/audit.json" 2>"$logs/audit.log"
	# The report as "<vulnerabilities> <warnings> <crates>", or nothing when it is not JSON.
	report=$(perl -MJSON::PP -e '
		local $/;
		my $j = eval { decode_json(<>) };
		exit 1 unless ref $j eq "HASH" && ref $j->{vulnerabilities} eq "HASH";
		my @found = @{$j->{vulnerabilities}{list} || []};
		my %seen;
		my @crates = grep { !$seen{$_}++ } map { "$_->{package}{name} $_->{package}{version}" } @found;
		my $warnings = 0;
		$warnings += @$_ for values %{$j->{warnings} || {}};
		print scalar(@found), " $warnings ", join(", ", @crates), "\n";
	' "$logs/audit.json")
	if [ -z "$report" ]; then
		fail "audit" "cargo audit gave no report, see $logs/audit.log"
	else
		read -r found warnings crates <<<"$report"
		if [ "$found" -ne 0 ]; then
			fail "audit" "$found vulnerability(ies) in $crates, see $logs/audit.json"
		else
			pass "audit" "0 vulnerability, $audit_ignored ignored, $warnings warning(s) in $logs/audit.json"
		fi
	fi
fi

if [ "$failed" -eq 0 ]; then
	echo "verdict  PASS"
	exit 0
fi
echo "verdict  FAIL"
exit 1
