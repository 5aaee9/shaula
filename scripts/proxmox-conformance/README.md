# Proxmox guest bootstrap checks

Run with Python 3 and Bash (Git for Windows Bash is supported):

```sh
python -m unittest discover -s scripts/proxmox-conformance -p 'test_*.py' -v
```

Set `SHAULA_TEST_BASH` to choose an explicit Bash executable. The tests read
the packaged `templates/proxmox/bootstrap.tftpl`; they do not keep a separate
copy of its control flow. Guest paths are redirected into a temporary
directory. Host operations (`chmod`, `chown`, `udevadm`, `blkid`, `findmnt`,
`umount`, `id`, `runuser`) and the Listener are command stubs, and a regular
fixture file substitutes for the seed block device.

These checks execute the shell ordering, failure propagation, durable startup
marker, JIT consumption, environment handoff, and argument/output checks with
a synthetic canary. A failed initialization or Listener does not get a second
registration attempt. The unit checks also keep cloud-final ordering,
nonblocking initial start, and the absence of reboot/restart installation.

This is mocked guest validation. It does not prove Linux UID/PAM behavior,
actual permissions, udev/systemd/cloud-init semantics, DHCP, Proxmox lifecycle,
GitHub JIT registration or cleanup. Those require the real platform acceptance
described in spec 0022. No hypervisor, Docker daemon or credentials are used.
