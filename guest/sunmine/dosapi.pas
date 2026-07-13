unit DosApi;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ Minimal DOS services used: time (2Ch), file create/write/close for STATE, exit. }

interface

type
  TDosTime = record
    Hour, Min, Sec, HSec: Byte;
  end;

procedure GetDosTime(var t: TDosTime);
procedure GetDosDate(var year: Word; var month, day: Byte);
function DosCreateFile(const path: String): Word; { returns handle or 0 on fail }
function DosOpenFile(const path: String; write: Boolean): Word;
procedure DosClose(handle: Word);
function DosWrite(handle: Word; buf: PByte; count: Word): Word;
function DosMkdir(const path: String): Boolean;
procedure DosExit(code: Byte);

implementation

procedure GetDosTime(var t: TDosTime);
begin
  asm
    mov ah, $2C
    int $21
    mov t.Hour, ch
    mov t.Min, cl
    mov t.Sec, dh
    mov t.HSec, dl
  end;
end;

procedure GetDosDate(var year: Word; var month, day: Byte);
var
  y: Word;
  m, d: Byte;
begin
  asm
    mov ah, $2A
    int $21
    mov y, cx
    mov m, dh
    mov d, dl
  end;
  year := y; month := m; day := d;
end;

function DosCreateFile(const path: String): Word;
var
  h: Word;
  p: array[0..127] of Char;
  i: Integer;
begin
  { convert pascal string to ascii z }
  for i := 1 to Length(path) do p[i-1] := path[i];
  p[Length(path)] := #0;
  h := 0;
  asm
    push ds
    mov dx, p
    mov ah, $3C
    mov cx, 0
    int $21
    jc @fail
    mov h, ax
    jmp @done
  @fail:
    mov h, 0
  @done:
    pop ds
  end;
  DosCreateFile := h;
end;

function DosOpenFile(const path: String; write: Boolean): Word;
var
  h: Word;
  p: array[0..127] of Char;
  i, mode: Integer;
begin
  for i := 1 to Length(path) do p[i-1] := path[i];
  p[Length(path)] := #0;
  if write then mode := $01 else mode := $00;
  h := 0;
  asm
    push ds
    mov dx, p
    mov ax, $3D00
    add al, byte ptr mode
    int $21
    jc @fail
    mov h, ax
    jmp @done
  @fail:
    mov h, 0
  @done:
    pop ds
  end;
  DosOpenFile := h;
end;

procedure DosClose(handle: Word);
begin
  asm
    mov bx, handle
    mov ah, $3E
    int $21
  end;
end;

function DosWrite(handle: Word; buf: PByte; count: Word): Word;
var
  written: Word;
begin
  written := 0;
  asm
    mov bx, handle
    mov cx, count
    mov dx, buf
    mov ah, $40
    int $21
    jc @fail
    mov written, ax
    jmp @done
  @fail:
    mov written, 0
  @done:
  end;
  DosWrite := written;
end;

function DosMkdir(const path: String): Boolean;
var
  ok: Boolean;
  p: array[0..127] of Char;
  i: Integer;
begin
  for i := 1 to Length(path) do p[i-1] := path[i];
  p[Length(path)] := #0;
  ok := False;
  asm
    push ds
    mov dx, p
    mov ah, $39
    int $21
    jc @fail
    mov ok, 1
  @fail:
    pop ds
  end;
  DosMkdir := ok;
end;

procedure DosExit(code: Byte);
begin
  asm
    mov ah, $4C
    mov al, code
    int $21
  end;
end;

end.
