# CK 0.10 Diagnostics

[简体中文](../zh-CN/reference/diagnostics.md)

This document is normative for diagnostic identifiers. Human-readable wording,
source excerpts, and caret widths may improve in `0.10.x`, but a code's phase and
meaning do not change. Diagnostics use
`file:line:column: error CKxxxx: message` followed by the source line and caret.

| Code | Phase | Trigger |
| --- | --- | --- |
| `CK0001` | Lexing | Invalid character or malformed numeric token. |
| `CK1001` | Parsing | Token sequence does not match CK grammar, including missing delimiters or an invalid statement/expression form. |
| `CK2001` | Type checking | Unknown variable. |
| `CK2002` | Type checking | Unknown function. |
| `CK2003` | Type checking | Unknown named type. |
| `CK2004` | Type checking | General type mismatch, invalid operator/operand, argument, field, index, return, or other semantic rule not assigned a narrower code. |
| `CK2005` | Type checking | Duplicate declaration, parameter, local, struct, or field. |
| `CK2006` | Type checking | Non-`bool` `if` or `while` condition. |
| `CK2007` | Type checking | Invalid assignment target. |
| `CK2008` | Type checking | Missing required return on a value-returning path. |
| `CK2009` | Type checking | `break` or `continue` outside an enclosing `while`. |
| `CK2010` | Type checking | Unreachable statement after a terminating statement in the same block. |
| `CK2011` | Type checking | Invalid `void` position or value/empty-return mismatch involving `void`. |
| `CK2012` | Type checking | Invalid slice element, construction, projection, index/range, assignment/call/return shape, or exported slice return. |
| `CK2013` | Type checking | Invalid, duplicate, or missing `main` entry for an executable consumer. A valid entry is internal, parameterless, and returns `void` or `i32`. |
| `CK2014` | Parsing / type checking | Invalid unsafe-function modifier or placement, contract placement, unsafe-block placement, executable-entry boundary, or unsafe call outside an explicit unsafe block. |
| `CK2015` | Type checking | Invalid, ill-typed, unsupported, or non-decidable closed contract expression, predicate, alignment, or effect target. |
| `CK2016` | KIR effect checking | A declared external-memory effect ceiling does not cover the inferred direct and transitive effects. |

One source file may produce multiple diagnostics in deterministic source order.
Exit status is nonzero when any error is reported. Backend/toolchain failures
are CLI errors rather than new `CKxxxx` semantic codes.
