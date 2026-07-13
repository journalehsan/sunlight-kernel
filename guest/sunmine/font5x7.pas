unit Font5x7;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ Compact 5x7 fixed font for Mode 13h direct draw.
  Supports 0-9 A-Z space : / - . ! and basic needed chars.
  Glyphs are bit-packed, 1 byte per row (5 bits used). }

interface

const
  FONT_W = 5;
  FONT_H = 7;
  FONT_GLYPH_BYTES = 7;

procedure DrawChar(x, y: Integer; ch: Char; color: Byte; fb: PByte);
procedure DrawString(x, y: Integer; const s: String; color: Byte; fb: PByte);

implementation

const
  { Glyph data for ASCII 32..90 range (space to Z), others map to space or digit. }
  GlyphData: array[0..59, 0..6] of Byte = (
    { ' ' 32 } (0,0,0,0,0,0,0),
    { '!' 33 } (4,4,4,4,0,4,0),
    { '"' 34 } (10,10,0,0,0,0,0),
    { '#' 35 } (10,31,10,31,10,0,0),
    { '$' 36 } (4,14,5,14,4,4,0),
    { '%' 37 } (17,2,4,8,17,0,0),
    { '&' 38 } (6,9,6,9,6,0,0),
    { ''' 39 } (4,4,0,0,0,0,0),
    { '(' 40 } (2,4,4,4,2,0,0),
    { ')' 41 } (8,4,4,4,8,0,0),
    { '*' 42 } (0,10,4,10,0,0,0),
    { '+' 43 } (0,4,14,4,0,0,0),
    { ',' 44 } (0,0,0,0,4,4,8),
    { '-' 45 } (0,0,14,0,0,0,0),
    { '.' 46 } (0,0,0,0,0,4,0),
    { '/' 47 } (1,2,4,8,16,0,0),
    { '0' 48 } (14,17,17,17,14,0,0),
    { '1' 49 } (4,12,4,4,14,0,0),
    { '2' 50 } (14,17,4,8,31,0,0),
    { '3' 51 } (14,17,6,17,14,0,0),
    { '4' 52 } (2,6,10,31,2,0,0),
    { '5' 53 } (31,16,30,1,30,0,0),
    { '6' 54 } (6,8,30,17,14,0,0),
    { '7' 55 } (31,1,2,4,8,0,0),
    { '8' 56 } (14,17,14,17,14,0,0),
    { '9' 57 } (14,17,15,1,6,0,0),
    { ':' 58 } (0,4,0,4,0,0,0),
    { ';' 59 } (0,4,0,4,4,8,0),
    { '<' 60 } (2,4,8,4,2,0,0),
    { '=' 61 } (0,14,0,14,0,0,0),
    { '>' 62 } (8,4,2,4,8,0,0),
    { '?' 63 } (14,17,2,4,4,0,4),
    { '@' 64 } (14,17,21,21,14,0,0),
    { 'A' 65 } (4,10,17,31,17,0,0),
    { 'B' 66 } (30,17,30,17,30,0,0),
    { 'C' 67 } (14,17,16,17,14,0,0),
    { 'D' 68 } (28,18,17,18,28,0,0),
    { 'E' 69 } (31,16,30,16,31,0,0),
    { 'F' 70 } (31,16,30,16,16,0,0),
    { 'G' 71 } (14,17,23,17,14,0,0),
    { 'H' 72 } (17,17,31,17,17,0,0),
    { 'I' 73 } (14,4,4,4,14,0,0),
    { 'J' 74 } (1,1,1,17,14,0,0),
    { 'K' 75 } (17,18,28,18,17,0,0),
    { 'L' 76 } (16,16,16,16,31,0,0),
    { 'M' 77 } (17,27,21,17,17,0,0),
    { 'N' 78 } (17,25,21,19,17,0,0),
    { 'O' 79 } (14,17,17,17,14,0,0),
    { 'P' 80 } (30,17,30,16,16,0,0),
    { 'Q' 81 } (14,17,17,21,14,1,0),
    { 'R' 82 } (30,17,30,18,17,0,0),
    { 'S' 83 } (14,16,14,1,14,0,0),
    { 'T' 84 } (31,4,4,4,4,0,0),
    { 'U' 85 } (17,17,17,17,14,0,0),
    { 'V' 86 } (17,17,10,10,4,0,0),
    { 'W' 87 } (17,17,21,21,10,0,0),
    { 'X' 88 } (17,10,4,10,17,0,0),
    { 'Y' 89 } (17,10,4,4,4,0,0),
    { 'Z' 90 } (31,2,4,8,31,0,0)
  );

function CharIndex(ch: Char): Integer;
var
  c: Integer;
begin
  c := Ord(UpCase(ch));
  if c = 32 then CharIndex := 0
  else if (c >= 48) and (c <= 57) then CharIndex := c - 32 { digits after space }
  else if (c >= 65) and (c <= 90) then CharIndex := c - 32
  else if c = 58 then CharIndex := 26  { : }
  else if c = 45 then CharIndex := 13  { - }
  else if c = 46 then CharIndex := 14  { . }
  else if c = 47 then CharIndex := 15  { / }
  else if c = 33 then CharIndex := 1   { ! }
  else CharIndex := 0; { default space }
end;

procedure DrawChar(x, y: Integer; ch: Char; color: Byte; fb: PByte);
var
  idx, row, bits, gx, gy, fbofs: Integer;
  b: Byte;
begin
  idx := CharIndex(ch);
  for row := 0 to FONT_H - 1 do
  begin
    b := GlyphData[idx, row];
    for gx := 0 to FONT_W - 1 do
    begin
      if (b and (1 shl (4 - gx))) <> 0 then
      begin
        gy := y + row;
        if (gy >= 0) and (gy < 200) and (x + gx >= 0) and (x + gx < 320) then
        begin
          fbofs := gy * 320 + (x + gx);
          fb[fbofs] := color;
        end;
      end;
    end;
  end;
end;

procedure DrawString(x, y: Integer; const s: String; color: Byte; fb: PByte);
var
  i: Integer;
begin
  for i := 1 to Length(s) do
    DrawChar(x + (i-1) * (FONT_W + 1), y, s[i], color, fb);
end;

end.
