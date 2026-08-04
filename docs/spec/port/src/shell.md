# src/shell.h

The umbrella header: build-wide typedefs and configuration. `pointer` is
the shell's spelling of `void *`, used for allocator return values.

> [spec:dash:def:shell.max-int-length-fn]
> static inline int max_int_length(int bytes)

> [spec:dash:sem:shell.max-int-length-fn]
> Return a buffer size guaranteed to hold the decimal rendering of any
> signed integer of `bytes` bytes, including sign and terminator.
> Computes `(bytes * 8 - 1) * 0.30102999566398119521 + 14` in floating
> point and truncates to `int`. The constant is log10(2), so
> `(bits - 1) * log10(2)` is the number of decimal digits the value range
> needs; the `+ 14` is a generous allowance covering the sign, the NUL,
> the truncation of the fractional digit count, and any prefix a caller
> may add. The result is an over-estimate by design — it is used to size
> stack buffers, so being slightly large is free and being short would be
> a buffer overflow.

> [spec:dash:def:shell.pointer]
> typedef void *pointer
