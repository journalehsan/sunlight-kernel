/* Retained compatibility wrapper for the 2026-11-12 updated-contract companion.
 *
 * The maintained original fixture, tools/native-c/tests/inet-socket.c, now
 * carries the supported TCP LT epoll contract directly: inert unattached
 * placeholders, rejected TCP ET/oneshot, and the unchanged socket/I/O,
 * exhaustion, rollback, duplication and thread assertions. Keeping this path
 * as an include wrapper avoids duplicated coverage without deleting the
 * historical companion evidence or its build/run entry point.
 */
#include "inet-socket.c"
