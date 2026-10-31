# The Tools pkg's postinstall library — ONE implementation, sourced by each
# component's postinstall (the builder copies this file into every component's
# scripts dir and bakes the version).
#
# THE LAWS (Tools doc §3A + the execution prompt §2):
# - No root, no sudo, ever: the pkg installs through
#   `installer -target CurrentUserHomeDirectory`, which runs everything as
#   the invoking user. These scripts never elevate.
# - The write-set is the user's home and NOTHING else: the payload anchor
#   (Library/Application Support/StewardMem/tools/<version>), the symlinks in
#   ~/.local/bin, and the 0700 modes on dirs WE create. ZERO bytes to the
#   operator's plist path and to the binary path it references.
# - Label-blind: these scripts never name the operator's service label. The
#   D1 facts arrive as `--fields` lines from stewardmem-detect (the detection
#   core's own binary — ONE implementation, two surfaces); a script that
#   cannot name a thing cannot clobber it.
# - Every overwriting act is named BEFORE it happens, and "do nothing" is
#   what the user already has: an existing binary is never replaced — the
#   new one lands beside it as <name>-<version>, and the hand-off says so.

VERSION="__VERSION__"
ANCHOR_REL="Library/Application Support/StewardMem/tools"
ANCHOR="$HOME/$ANCHOR_REL/$VERSION"
BIN_DIR="$HOME/.local/bin"
DETECT="$ANCHOR/stewardmem-detect"

PRESERVED=""
INSTALLED=""

# The D1 facts, through the detection core. A missing or failed detector is
# an ABSENT D1 (the plain install), never a guess — the anchor's own
# detector is in the payload we just installed, so absence here means the
# payload is broken, which shows up as a plain install and a broken link
# the user can see.
d1_field() {
    field=$1
    "$DETECT" --fields 2>/dev/null | sed -n "s/^${field}=//p" | head -n 1
}

# The one path the preserve law protects: whatever binary the operator's
# existing service references. Empty when there is no existing install.
referenced_binary_path() {
    d1_field "existing.binaryPath"
}

# Create the ~/.local/bin symlink for ONE tool, by the preserve law:
#   - the referenced path is NEVER touched (even when absent — creating it
#     would change what the service execs on its next spawn);
#   - an occupied path is never replaced — the new link lands beside it as
#     <name>-<version>, and the act is named;
#   - a free path takes <name> (the recommended default — there is nothing
#     to overwrite).
install_tool_link() {
    name=$1
    link="$BIN_DIR/$name"
    referenced=$(referenced_binary_path)
    if { [ -n "$referenced" ] && [ "$link" = "$referenced" ]; } || [ -e "$link" ] || [ -L "$link" ]; then
        link="$BIN_DIR/$name-$VERSION"
        PRESERVED="$PRESERVED $name"
    else
        INSTALLED="$INSTALLED $name"
    fi
    ln -sfn "$ANCHOR/$name" "$link"
}

# The mode laws, applied to OUR own artifacts only: 0700 on dirs we create
# (an existing dir is not ours to re-mode), 0755 on our binaries.
install_modes() {
    for dir in "$HOME/.local" "$BIN_DIR" "$HOME/Library/Application Support/StewardMem" "$HOME/$ANCHOR_REL" "$ANCHOR"; do
        if [ ! -e "$dir" ]; then
            mkdir -p "$dir"
            chmod 700 "$dir"
        fi
    done
    for tool in "$ANCHOR"/*; do
        [ -f "$tool" ] && chmod 755 "$tool"
    done
}

# The hand-off: every act named, with the versions and the paths — the Tools
# doc's sentence, printed to the installer log.
print_hand_off() {
    if [ -n "$PRESERVED" ]; then
        referenced=$(referenced_binary_path)
        existing_version=$(d1_field "existing.version")
        echo "HAND-OFF: an existing service manages these tools (version ${existing_version:-unknown} at ${referenced:-unknown path}):$PRESERVED"
        echo "HAND-OFF: the new version $VERSION is installed beside them as <name>-$VERSION under $BIN_DIR — the running service was never touched."
        echo "HAND-OFF: migrate when ready (see the README on the Tools image)."
    fi
    if [ -n "$INSTALLED" ]; then
        echo "INSTALLED:$INSTALLED -> symlinks in $BIN_DIR pointing into $ANCHOR"
    fi
}
