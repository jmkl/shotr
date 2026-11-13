# Install shotr for the current user and start it with Windows.
#   nu install.nu                 # install (uses target/release/shotr.exe)
#   nu install.nu --exe path.exe  # install a specific build
#   nu install.nu --uninstall     # remove it again
# No admin rights needed: copies to %LOCALAPPDATA%\Programs\shotr and adds a
# per-user Run registry entry (Windows starts it at login; nothing polls).

const RUN_KEY = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Run'

def stop-shotr [] {
    ps | where name =~ '(?i)^shotr(\.exe)?$' | each {|p| kill --force $p.pid } | ignore
    sleep 300ms # let Windows release the file lock
}

def main [
    --exe: path = "target/release/shotr.exe" # built executable to install
    --uninstall                              # remove shotr and its autostart entry
] {
    let dir = $env.LOCALAPPDATA | path join "Programs" "shotr"
    let dest = $dir | path join "shotr.exe"

    if $uninstall {
        stop-shotr
        ^reg delete $RUN_KEY /v shotr /f | ignore
        if ($dir | path exists) { rm -r $dir }
        print "shotr uninstalled."
        return
    }

    if not ($exe | path exists) {
        error make { msg: $"Executable not found: ($exe). Run `cargo build --release` first." }
    }

    stop-shotr
    mkdir $dir
    cp $exe $dest

    # Quoted path so folders with spaces work.
    ^reg add $RUN_KEY /v shotr /t REG_SZ /d $'"($dest)"' /f | ignore

    start $dest
    print $"Installed to ($dest). It now starts at login and is running in the tray."
}
