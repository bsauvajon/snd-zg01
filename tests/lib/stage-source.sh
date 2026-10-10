# Helper for the Arch packaging contract tests (sourced, not executed).
#
# Both PKGBUILDs fetch the project from `branch=main`.  On a pull request that
# branch does not carry the change under test - and for packaging that is
# introduced by the same pull request it carries nothing to build at all - so a
# package built from the default source either fails for the wrong reason or,
# worse, passes while testing main.  These tests build from a throwaway
# repository holding the tree at hand instead, through the PKGBUILD's
# ZG01_SOURCE override.

# zg01_stage_source_tree <repo_root> <parent_dir>
#
#   Creates "<parent_dir>/snd-zg01" as a git repository (branch main, one
#   commit) containing the working tree, and prints that path.  It stays a git
#   repository so pkgver() keeps working, and build outputs are left out:
#   a stale binary would let `make` do nothing and the contract prove nothing.
zg01_stage_source_tree() {
    local repo_root=$1
    local parent=$2

    mkdir -p "$parent/snd-zg01"
    # absolute: callers pass the result to git and makepkg, and a relative
    # path would depend on their working directory
    local stage
    stage=$(cd "$parent/snd-zg01" && pwd)
    tar -C "$repo_root" \
        --exclude=.git --exclude='*.o' --exclude='*.ko' --exclude='*.mod' \
        --exclude='*.mod.*' --exclude='*.cmd' --exclude=Module.symvers \
        --exclude=.tmp_versions --exclude='tools/zgctl' \
        -cf - . | tar -C "$stage" -xf -

    (
        cd "$stage"
        git init -q -b main
        git -c user.name=ci -c user.email=ci@example.invalid \
            -c commit.gpgsign=false add -A
        git -c user.name=ci -c user.email=ci@example.invalid \
            -c commit.gpgsign=false commit -qm 'tree under test'
    )

    printf '%s\n' "$stage"
}
