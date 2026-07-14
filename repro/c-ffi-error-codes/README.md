# Repro: C FFI panics on invalid input

## Issue

Unpatched `diode-file-bindings` used `.expect()` at the FFI boundary
(`diode_new_config`, `diode_send_file`, `diode_receive_files`). Invalid C strings
or pointers from embedders abort the whole process instead of returning an error
code.

## Automated repro

```bash
cargo test -p diode-file-bindings repro_
```

Key case: `repro_invalid_socket_address_returns_null_without_panic` passes a
non-parseable address; patched code returns `NULL`, unpatched code panics on
`expect("ip:port")`.

## Manual repro (unpatched master)

```c
#include "diode.h"

int main(void) {
    diode_new_config("not-an-address", 4096);  /* aborts on master */
    return 0;
}
```

## Expected with this fix

- Invalid address → `NULL` from `diode_new_config`
- Null/invalid config or paths → `0` from send/receive helpers
