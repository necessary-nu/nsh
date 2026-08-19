# Command-line metadata

The executable owns presentation that is useful before a shell exists. These
options are intentionally handled by the frontend rather than by the shell
language's option parser.

> [spec:nsh:req:cli.metadata-options]
> When its first invocation argument is `--help`, the `nsh` executable MUST
> write non-empty usage information to standard output and exit successfully
> without parsing shell input. When that argument is `--version`, it MUST
> write the executable name and package version to standard output and exit
> successfully. The same byte strings appearing after `-c` or a script
> operand MUST remain shell arguments rather than frontend options. `-h` MUST
> retain its existing shell-option meaning (`hashall`).
