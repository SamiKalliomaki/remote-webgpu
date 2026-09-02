# Fuzzing regressions

Inputs the fuzzer found, kept so they are replayed forever after.  Each one
crashed (or leaked) at the time it was found; `run.sh fuzz` replays the
whole directory through `build/fuzz_receive` before any new fuzzing starts,
so a regression fails the suite in a second rather than being rediscovered
after a few million executions -- or not at all.

| Input | What it caught |
| --- | --- |
| `hello-with-bad-alignment.bin` | A `ClientHello` reporting `minStorageBufferOffsetAlignment = 2304` -- not a power of two, and larger than the 256 the spec allows.  Alignments are divisors in the application's own size arithmetic, so an unclamped one (0 above all) is a division by zero or a corrupt layout.  Found by mutating the hello seed in handshake mode. |
| `present-done-leaks-pending-map.bin` | A bare `PresentDone` on a session with requests still outstanding.  Nothing crashed: LeakSanitizer caught the whole session leaking, because a request awaiting a reply holds a reference to the object it is about, and buffer -> device -> adapter -> request is a cycle.  Fixed by `wgpuRemoteAdapterAbandonRequests()`, which the transport calls when the client goes away. |

Add to this directory whenever the fuzzer finds something: copy the
artifact out of `artifacts/`, give it a name that says what it caught, and
add a row above.
