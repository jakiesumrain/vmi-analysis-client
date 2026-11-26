# Test fixtures

`host_key` and `host_key2.pub` are throwaway SSH host keys used only by the
in-process test server in `src/tunnel/tests.rs`.

They are committed so the tests do not need `ssh-keygen` at run time. They are
not used by the application and must never be trusted for anything else.
