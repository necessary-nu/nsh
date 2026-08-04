# Relationship to Other Documents

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in RFC 2119. This document
reproduces normative text from IEEE Std 1003.1-2024 (POSIX.1-2024),
Copyright © 2001-2024 The IEEE and The Open Group.

## 1.1.1 System Interfaces

> This subsection describes some of the features provided by the System
> Interfaces volume of POSIX.1-2024 that are assumed to be globally available on
> all systems conforming to this volume of POSIX.1-2024. This subsection does
> not attempt to detail all of the features defined in the System Interfaces
> volume of POSIX.1-2024 that are required by all of the utilities defined in
> this volume of POSIX.1-2024; the utility and function descriptions point out
> additional functionality required to provide the corresponding specific
> features needed by each.
>
> Source: XCU 1.1.1 System Interfaces — utilities/V3_chap01.html#tag_18_01_01

> The following subsections describe frequently used concepts. Many of these
> concepts are described in the Base Definitions volume of POSIX.1-2024. Utility
> and function description statements override these defaults when appropriate.
>
> Source: XCU 1.1.1 System Interfaces — utilities/V3_chap01.html#tag_18_01_01

### 1.1.1.1 Process Attributes

> The following process attributes, as described in the System Interfaces volume
> of POSIX.1-2024, are assumed to be supported for all processes in this volume
> of POSIX.1-2024:
>
> | | |
> |---|---|
> | Controlling Terminal | Real Group ID |
> | Current Working Directory | Real User ID |
> | Effective Group ID | Root Directory |
> | Effective User ID | Saved Set-Group-ID |
> | File Descriptors | Saved Set-User-ID |
> | File Mode Creation Mask | Session Membership |
> | Process Group ID | Supplementary Group IDs |
> | Process ID | |
>
> Source: XCU 1.1.1.1 Process Attributes — utilities/V3_chap01.html#tag_18_01_01_01

> [spec:posix:req:xcurel.process-attributes-additional]
> A conforming implementation may include additional process attributes.
>
> Source: XCU 1.1.1.1 Process Attributes — utilities/V3_chap01.html#tag_18_01_01_01

### 1.1.1.2 Concurrent Execution of Processes

> [spec:posix:req:xcurel.concurrent-execution]
> The following functionality of the fork() function defined in the System
> Interfaces volume of POSIX.1-2024 shall be available on all systems conforming
> to this volume of POSIX.1-2024:
>
> 1. Independent processes shall be capable of executing independently without
>    either process terminating.
> 2. A process shall be able to create a new process with all of the attributes
>    referenced in 1.1.1.1 Process Attributes, determined according to the
>    semantics of a call to the fork() function defined in the System Interfaces
>    volume of POSIX.1-2024 followed by a call in the child process to one of
>    the exec functions defined in the System Interfaces volume of POSIX.1-2024.
>
> Source: XCU 1.1.1.2 Concurrent Execution of Processes — utilities/V3_chap01.html#tag_18_01_01_02

### 1.1.1.3 File Access Permissions

> [spec:posix:req:xcurel.file-access-permissions]
> The file access control mechanism described by XBD 4.7 File Access Permissions
> shall apply to all files on an implementation conforming to this volume of
> POSIX.1-2024.
>
> Source: XCU 1.1.1.3 File Access Permissions — utilities/V3_chap01.html#tag_18_01_01_03

### 1.1.1.4 File Read, Write, and Creation

> [spec:posix:req:xcurel.file-create-if-absent]
> If a file that does not exist is to be written, it shall be created as
> described below, unless the utility description states otherwise.
>
> Source: XCU 1.1.1.4 File Read, Write, and Creation — utilities/V3_chap01.html#tag_18_01_01_04

> [spec:posix:req:xcurel.file-creation-attributes]
> When a file that does not exist is created, the following features defined in
> the System Interfaces volume of POSIX.1-2024 shall apply unless the utility or
> function description states otherwise:
>
> 1. The user ID of the file shall be set to the effective user ID of the
>    calling process.
> 2. The group ID of the file shall be set to the effective group ID of the
>    calling process or the group ID of the directory in which the file is being
>    created.
> 3. If the file is a regular file, the permission bits of the file shall be set
>    to: `S_IROTH | S_IWOTH | S_IRGRP | S_IWGRP | S_IRUSR | S_IWUSR`
>
>    (see the description of *File Modes* in XBD 14. Headers, `<sys/stat.h>`)
>    except that the bits specified by the file mode creation mask of the
>    process shall be cleared. If the file is a directory, the permission bits
>    shall be set to: `S_IRWXU | S_IRWXG | S_IRWXO`
>
>    except that the bits specified by the file mode creation mask of the
>    process shall be cleared.
> 4. The last data access, last data modification, and last file status change
>    timestamps of the file shall be updated as specified in XBD 4.12 File Times
>    Update.
> 5. If the file is a directory, it shall be an empty directory; otherwise, the
>    file shall have length zero.
> 6. If the file is a symbolic link, the effect shall be undefined unless the
>    {POSIX2_SYMLINKS} variable is in effect for the directory in which the
>    symbolic link would be created.
> 7. Unless otherwise specified, the file created shall be a regular file.
>
> Source: XCU 1.1.1.4 File Read, Write, and Creation — utilities/V3_chap01.html#tag_18_01_01_04

> [spec:posix:req:xcurel.file-create-existing-actions]
> When an attempt is made to create a file that already exists, the utility
> shall take the action indicated in Actions when Creating a File that Already
> Exists corresponding to the type of the file the utility is trying to create
> and the type of the existing file, unless the utility description states
> otherwise.
>
> Table: Actions when Creating a File that Already Exists
>
> The column headings B through T are the New Type, given with the same
> single-letter type codes as the Existing Type rows.
>
> | Existing Type | | B | C | D | F | L | M | P | Q | R | S | T | Function Creating New |
> |---|---|---|---|---|---|---|---|---|---|---|---|---|---|
> | B | Block Special | F | F | F | F | F | U | U | U | OF | U | U | `mknod()**` |
> | C | Character Special | F | F | F | F | F | U | U | U | OF | U | U | `mknod()**` |
> | D | Directory | F | F | F | F | F | — | — | — | F | — | U | `mkdir()` |
> | F | FIFO Special File | F | F | F | F | F | — | — | — | O | — | U | `mkfifo()` |
> | L | Symbolic Link | F | F | F | F | F | — | — | — | FL | — | U | `symlink()` |
> | M | Shared Memory | F | F | F | F | F | — | — | — | — | — | U | `shm_open()` |
> | P | Semaphore | F | F | F | F | F | — | — | — | — | — | U | `sem_open()` |
> | Q | Message Queue | F | F | F | F | F | — | — | — | — | — | U | `mq_open()` |
> | R | Regular File | F | F | F | F | F | — | — | — | RF | — | U | `open()` |
> | S | Socket | F | F | F | F | F | — | — | — | — | — | U | `bind()` |
> | T | Typed Memory | F | F | F | F | F | U | U | U | U | U | U | `*` |
>
> Source: XCU 1.1.1.4 File Read, Write, and Creation — utilities/V3_chap01.html#tag_18_01_01_04

> [spec:posix:def:xcurel.file-create-existing-codes]
> The following codes are used in Actions when Creating a File that Already
> Exists:
>
> **F**
>
> Fail. The attempt to create the new file shall fail and the utility shall
> either continue with its operation or exit immediately with an exit status
> that indicates an error occurred, depending on the description of the utility.
>
> **FL**
>
> Follow link. Unless otherwise specified, the symbolic link shall be followed
> as specified for pathname resolution, and the operation performed shall be as
> if the target of the symbolic link (after all resolution) had been named. If
> the target of the symbolic link does not exist, it shall be as if that
> nonexistent target had been named directly.
>
> **O**
>
> Open FIFO. When attempting to create a regular file, and the existing file is
> a FIFO special file:
>
> 1. If the FIFO is not already open for reading, the attempt shall block until
>    the FIFO is opened for reading.
> 2. Once the FIFO is open for reading, the utility shall open the FIFO for
>    writing and continue with its operation.
>
> **OF**
>
> The named file shall be opened with the consequences defined for that file
> type.
>
> **RF**
>
> Regular file. When attempting to create a regular file, and the existing file
> is a regular file:
>
> 1. The user ID, group ID, and permission bits of the file shall not be
>    changed.
> 2. The file shall be truncated to zero length.
> 3. The last data modification and last file status change timestamps shall be
>    marked for update.
>
> **—**
>
> The effect is implementation-defined unless specified by the utility
> description.
>
> **U**
>
> The effect is unspecified unless specified by the utility description.
>
> **`*`**
>
> There is no portable way to create a file of this type.
>
> **`**`**
>
> Not portable.
>
> Source: XCU 1.1.1.4 File Read, Write, and Creation — utilities/V3_chap01.html#tag_18_01_01_04

> [spec:posix:req:xcurel.file-append-mode]
> When a file is to be appended, the file shall be opened in a manner equivalent
> to using the O_APPEND flag, without the O_TRUNC flag, in the open() function
> defined in the System Interfaces volume of POSIX.1-2024.
>
> Source: XCU 1.1.1.4 File Read, Write, and Creation — utilities/V3_chap01.html#tag_18_01_01_04

> [spec:posix:req:xcurel.file-open-access-mode]
> When a file is to be read or written, the file shall be opened with an access
> mode corresponding to the operation to be performed. If file access
> permissions deny access, the requested operation shall fail.
>
> Source: XCU 1.1.1.4 File Read, Write, and Creation — utilities/V3_chap01.html#tag_18_01_01_04

### 1.1.1.5 File Removal

> [spec:posix:req:xcurel.file-removal-active-directory]
> When a directory that is the root directory or current working directory of
> any process is removed, the effect is implementation-defined.
>
> Source: XCU 1.1.1.5 File Removal — utilities/V3_chap01.html#tag_18_01_01_05

> [spec:posix:req:xcurel.file-removal-effects]
> If file access permissions deny access, the requested operation shall fail.
> Otherwise, when a file is removed:
>
> 1. Its directory entry shall be removed from the file system.
> 2. The link count of the file shall be decremented.
> 3. If the file is an empty directory (see XBD 3.119 Empty Directory):
>    1. If no process has the directory open, the space occupied by the
>       directory shall be freed and the directory shall no longer be
>       accessible.
>    2. If one or more processes have the directory open, the directory contents
>       shall be preserved until all references to the file have been closed.
> 4. If the file is a directory that is not empty, the last file status change
>    timestamp shall be marked for update.
> 5. If the file is not a directory:
>    1. If the link count becomes zero:
>       1. If no process has the file open, the space occupied by the file shall
>          be freed and the file shall no longer be accessible.
>       2. If one or more processes have the file open, the file contents shall
>          be preserved until all references to the file have been closed.
>    2. If the link count is not reduced to zero, the last file status change
>       timestamp shall be marked for update.
> 6. The last data modification and last file status change timestamps of the
>    containing directory shall be marked for update.
>
> Source: XCU 1.1.1.5 File Removal — utilities/V3_chap01.html#tag_18_01_01_05

### 1.1.1.6 File Time Values

> [spec:posix:req:xcurel.file-time-values]
> All files shall have the three time values described by XBD 4.12 File Times
> Update.
>
> Source: XCU 1.1.1.6 File Time Values — utilities/V3_chap01.html#tag_18_01_01_06

### 1.1.1.7 File Contents

> When a reference is made to the contents of a file, *pathname*, this means the
> equivalent of all of the data placed in the space pointed to by *buf* when
> performing the read() function calls in the following operations defined in
> the System Interfaces volume of POSIX.1-2024:
>
> ```
> while (read (fildes, buf, nbytes) > 0)
>     ;
> ```
>
> If the file is indicated by a pathname *pathname*, the file descriptor shall
> be determined by the equivalent of the following operation defined in the
> System Interfaces volume of POSIX.1-2024:
>
> ```
> fildes = open (pathname, O_RDONLY);
> ```
>
> Source: XCU 1.1.1.7 File Contents — utilities/V3_chap01.html#tag_18_01_01_07

> [spec:posix:req:xcurel.file-contents-nbytes]
> The value of *nbytes* in the above sequence is unspecified; if the file is of
> a type where the data returned by read() would vary with different values, the
> value shall be one that results in the most data being returned.
>
> Source: XCU 1.1.1.7 File Contents — utilities/V3_chap01.html#tag_18_01_01_07

> [spec:posix:sem:xcurel.file-contents-read-error]
> If the read() function calls would return an error, it is unspecified whether
> the contents of the file are considered to include any data from offsets in
> the file beyond where the error would be returned.
>
> Source: XCU 1.1.1.7 File Contents — utilities/V3_chap01.html#tag_18_01_01_07

### 1.1.1.8 Pathname Resolution

> [spec:posix:req:xcurel.pathname-resolution]
> The pathname resolution algorithm, described by XBD 4.16 Pathname Resolution,
> shall be used by implementations conforming to this volume of POSIX.1-2024;
> see also XBD 4.8 File Hierarchy.
>
> Source: XCU 1.1.1.8 Pathname Resolution — utilities/V3_chap01.html#tag_18_01_01_08

### 1.1.1.9 Changing the Current Working Directory

> [spec:posix:req:xcurel.change-cwd]
> When the current working directory (see XBD 3.94 Current Working Directory) is
> to be changed, unless the utility or function description states otherwise,
> the operation shall succeed unless a call to the chdir() function defined in
> the System Interfaces volume of POSIX.1-2024 would fail when invoked with the
> new working directory pathname as its argument.
>
> The cd utility changes the current working directory subject to this rule;
> see `[spec:posix:req:builtin.cd.step10-chdir]` in `builtins-process.md`.
>
> Source: XCU 1.1.1.9 Changing the Current Working Directory — utilities/V3_chap01.html#tag_18_01_01_09

### 1.1.1.10 Establish the Locale

> [spec:posix:req:xcurel.establish-locale]
> The functionality of the setlocale() function defined in the System Interfaces
> volume of POSIX.1-2024 shall be available on all systems conforming to this
> volume of POSIX.1-2024; that is, utilities that require the capability of
> establishing an international operating environment shall be permitted to set
> the specified category of the international environment.
>
> Source: XCU 1.1.1.10 Establish the Locale — utilities/V3_chap01.html#tag_18_01_01_10

### 1.1.1.11 Actions Equivalent to Functions

> Some utility descriptions specify that a utility performs actions equivalent
> to a function defined in the System Interfaces volume of POSIX.1-2024. Such
> specifications require only that the external effects be equivalent, not that
> any effect within the utility and visible only to the utility be equivalent.
>
> Source: XCU 1.1.1.11 Actions Equivalent to Functions — utilities/V3_chap01.html#tag_18_01_01_11

## 1.1.2 Concepts Derived from the ISO C Standard

> [spec:posix:req:xcurel.iso-c-concepts]
> Some of the standard utilities perform complex data manipulation using their
> own procedure and arithmetic languages, as defined in their EXTENDED
> DESCRIPTION or OPERANDS sections. Unless otherwise noted, the arithmetic and
> semantic concepts (precision, type conversion, control flow, and so on) shall
> be equivalent to those defined in the ISO C standard, as described in the
> following sections. Note that there is no requirement that the standard
> utilities be implemented in any particular programming language.
>
> Source: XCU 1.1.2 Concepts Derived from the ISO C Standard — utilities/V3_chap01.html#tag_18_01_02

### 1.1.2.1 Arithmetic Precision and Operations

> [spec:posix:req:xcurel.arithmetic-precision]
> Integer variables and constants, including the values of operands and
> option-arguments, used by the standard utilities listed in this volume of
> POSIX.1-2024 shall be implemented as equivalent to the ISO C standard **signed
> long** data type; floating point shall be implemented as equivalent to the ISO
> C standard **double** type. Conversions between types shall be as described in
> the ISO C standard.
>
> Source: XCU 1.1.2.1 Arithmetic Precision and Operations — utilities/V3_chap01.html#tag_18_01_02_01

> [spec:posix:req:xcurel.arithmetic-variable-initialization]
> All variables shall be initialized to zero if they are not otherwise assigned
> by the input to the application.
>
> Source: XCU 1.1.2.1 Arithmetic Precision and Operations — utilities/V3_chap01.html#tag_18_01_02_01

> [spec:posix:req:xcurel.arithmetic-operators]
> Arithmetic operators and control flow keywords shall be implemented as
> equivalent to those in the cited ISO C standard section, as listed in Selected
> ISO C Standard Operators and Control Flow Keywords.
>
> **Note:** The comma operator (section 6.5.17 of the ISO C standard) is
> intentionally not included in the table. It need not be supported by
> implementations.
>
> Table: Selected ISO C Standard Operators and Control Flow Keywords
>
> | Operation | ISO C Standard Equivalent Reference |
> |---|---|
> | `()` | Section 6.5.1, Primary Expressions |
> | `postfix ++`, `postfix --` | Section 6.5.2, Postfix Operators |
> | `unary +`, `unary -`, `prefix ++`, `prefix --`, `~`, `!`, `sizeof()` | Section 6.5.3, Unary Operators |
> | `*`, `/`, `%` | Section 6.5.5, Multiplicative Operators |
> | `+`, `-` | Section 6.5.6, Additive Operators |
> | `<<`, `>>` | Section 6.5.7, Bitwise Shift Operators |
> | `<`, `<=`, `>`, `>=` | Section 6.5.8, Relational Operators |
> | `==`, `!=` | Section 6.5.9, Equality Operators |
> | `&` | Section 6.5.10, Bitwise AND Operator |
> | `^` | Section 6.5.11, Bitwise Exclusive OR Operator |
> | `\|` | Section 6.5.12, Bitwise Inclusive OR Operator |
> | `&&` | Section 6.5.13, Logical AND Operator |
> | `\|\|` | Section 6.5.14, Logical OR Operator |
> | `expr?expr:expr` | Section 6.5.15, Conditional Operator |
> | `=`, `*=`, `/=`, `%=`, `+=`, `-=`, `<<=`, `>>=`, `&=`, `^=`, `\|=` | Section 6.5.16, Assignment Operators |
> | `if ()`, `if () ... else`, `switch ()` | Section 6.8.4, Selection Statements |
> | `while ()`, `do ... while ()`, `for ()` | Section 6.8.5, Iteration Statements |
> | `goto`, `continue`, `break`, `return` | Section 6.8.6, Jump Statements |
>
> Shell arithmetic expansion requires only a subset of this table; see
> `[spec:posix:req:expand.arith-evaluation]` in `expansion.md` for the
> exceptions that apply to `$(( ))`.
>
> Source: XCU 1.1.2.1 Arithmetic Precision and Operations — utilities/V3_chap01.html#tag_18_01_02_01

> [spec:posix:req:xcurel.arithmetic-expression-evaluation]
> The evaluation of arithmetic expressions shall be equivalent to that described
> in Section 6.5, Expressions, of the ISO C standard.
>
> Source: XCU 1.1.2.1 Arithmetic Precision and Operations — utilities/V3_chap01.html#tag_18_01_02_01

### 1.1.2.2 Mathematical Functions

> [spec:posix:req:xcurel.mathematical-functions]
> Any mathematical functions with the same names as those in the following
> sections of the ISO C standard:
>
> - Section 7.12, Mathematics, `<math.h>`
> - Section 7.22.2, Pseudo-Random Sequence Generation Functions
>
> shall be implemented to return the results equivalent to those returned from a
> call to the corresponding function described in the ISO C standard.
>
> Source: XCU 1.1.2.2 Mathematical Functions — utilities/V3_chap01.html#tag_18_01_02_02
