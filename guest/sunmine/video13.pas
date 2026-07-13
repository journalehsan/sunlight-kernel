unit Video13;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ Mode 13h 320x200x256 direct A000:0000 access + VGA DAC palette programming. }

interface

const
  SCREEN_W = 320;
  SCREEN_H = 200;
  FB_SEG = $A000;

type
  PByte = ^Byte;

procedure SetVideoMode13;
procedure SetVideoMode03;
procedure SetPixel(x, y: Integer; color: Byte);
procedure FillRect(x, y, w, h: Integer; color: Byte);
procedure ClearScreen(color: Byte);
procedure ProgramDAC(index, r6, g6, b6: Byte);
procedure InstallSunlightPalette;

implementation

procedure SetVideoMode13;
begin
  asm
    mov ax, $0013
    int $10
  end;
end;

procedure SetVideoMode03;
begin
  asm
    mov ax, $0003
    int $10
  end;
end;

procedure SetPixel(x, y: Integer; color: Byte);
var
  fb: PByte;
  ofs: Word;
begin
  if (x < 0) or (x >= SCREEN_W) or (y < 0) or (y >= SCREEN_H) then Exit;
  fb := Ptr(FB_SEG, 0);
  ofs := Word(y) * SCREEN_W + Word(x);
  fb[ofs] := color;
end;

procedure FillRect(x, y, w, h: Integer; color: Byte);
var
  fb: PByte;
  row, col: Integer;
  base: Word;
begin
  fb := Ptr(FB_SEG, 0);
  for row := 0 to h-1 do
  begin
    if (y + row < 0) or (y + row >= SCREEN_H) then Continue;
    base := Word(y + row) * SCREEN_W + Word(x);
    for col := 0 to w-1 do
    begin
      if (x + col >= 0) and (x + col < SCREEN_W) then
        fb[base + Word(col)] := color;
    end;
  end;
end;

procedure ClearScreen(color: Byte);
var
  fb: PByte;
  i: Integer;
begin
  fb := Ptr(FB_SEG, 0);
  for i := 0 to (SCREEN_W * SCREEN_H) - 1 do
    fb[i] := color;
end;

{ Program one DAC entry. Caller must have entered mode 13h. }
procedure ProgramDAC(index, r6, g6, b6: Byte);
begin
  asm
    mov dx, $03C8
    mov al, index
    out dx, al
    mov dx, $03C9
    mov al, r6
    out dx, al
    mov al, g6
    out dx, al
    mov al, b6
    out dx, al
  end;
end;

procedure InstallSunlightPalette;
var
  i: Byte;
begin
  { Deep navy bg 0, panel 1, accent orange ~2, revealed 3, hidden 4, etc. }
  { 0: deep navy bg }
  ProgramDAC(0, 3, 5, 12);
  { 1: dark blue-gray panel }
  ProgramDAC(1, 8, 10, 14);
  { 2: Sunlight orange accent }
  ProgramDAC(2, 48, 24, 0);
  { 3: warm light gray revealed }
  ProgramDAC(3, 52, 52, 48);
  { 4: cool gray hidden }
  ProgramDAC(4, 28, 30, 34);
  { 5: border highlight pale }
  ProgramDAC(5, 55, 55, 55);
  { 6: border shadow }
  ProgramDAC(6, 12, 12, 14);
  { 7: mine dark red/black }
  ProgramDAC(7, 20, 0, 0);
  { 8: flag orange/red }
  ProgramDAC(8, 55, 12, 8);
  { 9: text warm white }
  ProgramDAC(9, 58, 56, 50);
  { 10: number colors 1-8 }
  ProgramDAC(10, 0, 20, 55);   { 1 blue }
  ProgramDAC(11, 0, 40, 0);    { 2 green }
  ProgramDAC(12, 55, 0, 0);    { 3 red }
  ProgramDAC(13, 0, 0, 40);    { 4 dark blue }
  ProgramDAC(14, 40, 0, 40);   { 5 purple }
  ProgramDAC(15, 0, 35, 35);   { 6 cyan }
  ProgramDAC(16, 0, 0, 0);     { 7 black for mine detail }
  ProgramDAC(17, 60, 60, 20);  { 8 yellow }

  { Fill remaining with reasonable grays for safety }
  for i := 18 to 31 do
    ProgramDAC(i, 16 + (i-18)*2, 16 + (i-18)*2, 18 + (i-18)*2);
  for i := 32 to 255 do
    ProgramDAC(i, 20, 22, 26);
end;

end.
