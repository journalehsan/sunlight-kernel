unit Video;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ Text mode 03h (80x25) direct B8000 access for Chronos.
  Each screen cell is 2 bytes: character at even offset, attribute at odd.
  B800:0000 = col 0 row 0, B800:0002 = col 1 row 0, etc.
  Row N starts at offset N * 80 * 2 = N * 160. }

interface

const
  SCREEN_COLS = 80;
  SCREEN_ROWS = 25;
  VID_SEG = $B800;

type
  PCharAttr = ^Word;  { 16-bit B8000 cell: low=char, high=attr }

function VidPtr(row, col: Integer): PCharAttr;

procedure SetVideoMode03;
procedure PutCell(row, col: Integer; ch: Char; attr: Byte);
procedure PutText(row, col: Integer; const s: String; attr: Byte);
procedure FillRow(row, startCol, endCol: Integer; ch: Char; attr: Byte);
procedure FillRegion(row1, col1, row2, col2: Integer; ch: Char; attr: Byte);
procedure ClearScreen;

{ Standard DOS text attributes }
const
  ATTR_TEXT    = $07;  { white on black }
  ATTR_TITLE   = $1E;  { yellow on blue }
  ATTR_STATUS  = $0F;  { bright white on black }
  ATTR_HIDDEN  = $70;  { black on gray (with 0xDB char = solid gray) }
  ATTR_REVEALED= $07;  { white on black }
  ATTR_FLAG    = $4F;  { bright white on red }
  ATTR_FLAG_WRONG=$4C; { bright red on red }
  ATTR_MINE    = $0F;  { bright white on black (for '*') }
  ATTR_MINE_BG = $40;  { red background }
  ATTR_WON     = $2F;  { bright white on green }
  ATTR_LOST    = $4F;  { bright white on red }
  { Number colors 1-8 }
  ATTR_NUM1 = $09;  { blue }
  ATTR_NUM2 = $0A;  { green }
  ATTR_NUM3 = $0C;  { red }
  ATTR_NUM4 = $01;  { dark blue }
  ATTR_NUM5 = $05;  { magenta }
  ATTR_NUM6 = $03;  { cyan }
  ATTR_NUM7 = $07;  { gray }
  ATTR_NUM8 = $0E;  { yellow }

implementation

function VidPtr(row, col: Integer): PCharAttr;
begin
  VidPtr := Ptr(VID_SEG, (row * SCREEN_COLS + col) * 2);
end;

procedure SetVideoMode03; assembler;
asm
  mov ax, $0003
  int $10
end;

procedure PutCell(row, col: Integer; ch: Char; attr: Byte);
var
  p: PCharAttr;
begin
  if (row < 0) or (row >= SCREEN_ROWS) then Exit;
  if (col < 0) or (col >= SCREEN_COLS) then Exit;
  p := VidPtr(row, col);
  p^ := Word(ch) or (Word(attr) shl 8);
end;

procedure PutText(row, col: Integer; const s: String; attr: Byte);
var
  i: Integer;
begin
  if (row < 0) or (row >= SCREEN_ROWS) then Exit;
  for i := 1 to Length(s) do
  begin
    if (col + i - 1 >= SCREEN_COLS) then Break;
    PutCell(row, col + i - 1, s[i], attr);
  end;
end;

procedure FillRow(row, startCol, endCol: Integer; ch: Char; attr: Byte);
var
  c: Integer;
begin
  if (row < 0) or (row >= SCREEN_ROWS) then Exit;
  for c := startCol to endCol do
    PutCell(row, c, ch, attr);
end;

procedure FillRegion(row1, col1, row2, col2: Integer; ch: Char; attr: Byte);
var
  r, c: Integer;
begin
  for r := row1 to row2 do
    FillRow(r, col1, col2, ch, attr);
end;

procedure ClearScreen;
var
  r: Integer;
begin
  for r := 0 to SCREEN_ROWS - 1 do
    FillRow(r, 0, SCREEN_COLS - 1, ' ', ATTR_TEXT);
end;

end.
