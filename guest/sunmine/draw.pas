unit Draw;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ All drawing of the Minesweeper UI into A0000 by the guest. }

interface

uses Video13, Font5x7, Game;

const
  CELL_SIZE = 14;
  BOARD_X = 38;   { centered for 9*14 = 126,  (320-126)/2 ~ 97? adjust }
  BOARD_Y = 56;
  TITLE_Y = 6;
  STATUS_Y = 26;
  HINT_Y = 184;

procedure DrawFullUI(fb: PByte);
procedure DrawCell(x, y, cx, cy: Integer; revealed, flagged, isMine: Boolean; neighbor: Byte; lost: Boolean);
procedure DrawBanner(msg: String; good: Boolean);

implementation

const
  COL_BG = 0;
  COL_PANEL = 1;
  COL_ACCENT = 2;
  COL_REVEALED = 3;
  COL_HIDDEN = 4;
  COL_HILITE = 5;
  COL_SHADOW = 6;
  COL_MINE = 7;
  COL_FLAG = 8;
  COL_TEXT = 9;
  { 10-17 for numbers }

procedure DrawCell(x, y, cx, cy: Integer; revealed, flagged, isMine: Boolean; neighbor: Byte; lost: Boolean);
var
  px, py, c, nc: Integer;
  numc: Byte;
begin
  px := x + cx * CELL_SIZE;
  py := y + cy * CELL_SIZE;
  if revealed then
  begin
    FillRect(px, py, CELL_SIZE, CELL_SIZE, COL_REVEALED);
    { inner bevel thin }
    if neighbor > 0 then
    begin
      case neighbor of
        1: numc := 10;
        2: numc := 11;
        3: numc := 12;
        4: numc := 13;
        5: numc := 14;
        6: numc := 15;
        7: numc := 16;
      else
        numc := 17;
      end;
      DrawString(px + 4, py + 3, Chr(Ord('0') + neighbor), numc, Ptr(FB_SEG, 0));
    end;
  end
  else
  begin
    FillRect(px, py, CELL_SIZE, CELL_SIZE, COL_HIDDEN);
    { simple bevel }
    { top/left hilite }
    FillRect(px, py, CELL_SIZE, 2, COL_HILITE);
    FillRect(px, py, 2, CELL_SIZE, COL_HILITE);
    { bottom/right shadow }
    FillRect(px, py + CELL_SIZE - 2, CELL_SIZE, 2, COL_SHADOW);
    FillRect(px + CELL_SIZE - 2, py, 2, CELL_SIZE, COL_SHADOW);
    if flagged then
    begin
      { draw flag }
      FillRect(px + 4, py + 3, 6, 2, COL_FLAG);
      FillRect(px + 4, py + 5, 2, 6, COL_FLAG);
      FillRect(px + 3, py + 10, 8, 2, COL_TEXT);
    end;
  end;
  if lost and isMine and revealed then
  begin
    { detonated mine highlight }
    FillRect(px+2, py+2, CELL_SIZE-4, CELL_SIZE-4, COL_MINE);
    DrawString(px+4, py+4, '*', COL_TEXT, Ptr(FB_SEG,0));
  end
  else if lost and isMine and (not revealed) then
  begin
    { show remaining mines }
    FillRect(px+3, py+3, CELL_SIZE-6, CELL_SIZE-6, COL_MINE);
  end;
  if lost and flagged and (not isMine) then
  begin
    { incorrect flag mark }
    DrawString(px + 3, py + 3, 'X', COL_ACCENT, Ptr(FB_SEG,0));
  end;
end;

procedure DrawBoard(fb: PByte);
var
  cx, cy: Integer;
begin
  for cy := 0 to BOARD_H - 1 do
    for cx := 0 to BOARD_W - 1 do
      DrawCell(BOARD_X, BOARD_Y, cx, cy,
        Board[cy, cx].IsRevealed,
        Board[cy, cx].IsFlagged,
        Board[cy, cx].IsMine,
        Board[cy, cx].Neighbor,
        GameLost);
end;

procedure DrawStatus(fb: PByte);
var
  s: String;
  best: Word;
begin
  { mine counter }
  s := 'MINES ';
  if RemainingMines < 10 then s := s + '00' else if RemainingMines < 100 then s := s + '0';
  s := s + IntToStr(RemainingMines); { note: FPC shortstr no IntToStr in tp? use manual }
  DrawString(20, STATUS_Y, 'MINES', COL_ACCENT, fb);
  DrawString(20 + 6*6, STATUS_Y, '  ', COL_TEXT, fb);
  { manual digits for counter }
  { For simplicity draw two digits }
  DrawString(70, STATUS_Y, Chr(Ord('0') + (RemainingMines div 10)), COL_TEXT, fb);
  DrawString(76, STATUS_Y, Chr(Ord('0') + (RemainingMines mod 10)), COL_TEXT, fb);

  { timer }
  DrawString(160, STATUS_Y, 'TIME', COL_ACCENT, fb);
  DrawString(200, STATUS_Y, Chr(Ord('0') + (ElapsedSec div 100)), COL_TEXT, fb);
  DrawString(206, STATUS_Y, Chr(Ord('0') + ((ElapsedSec div 10) mod 10)), COL_TEXT, fb);
  DrawString(212, STATUS_Y, Chr(Ord('0') + (ElapsedSec mod 10)), COL_TEXT, fb);

  { best time }
  best := 0; { caller may load }
  DrawString(250, STATUS_Y, 'BEST', COL_ACCENT, fb);
  if best > 0 then
  begin
    DrawString(280, STATUS_Y, Chr(Ord('0')+(best div 100)), COL_TEXT, fb);
    DrawString(286, STATUS_Y, Chr(Ord('0')+((best div 10) mod 10)), COL_TEXT, fb);
    DrawString(292, STATUS_Y, Chr(Ord('0')+(best mod 10)), COL_TEXT, fb);
  end
  else
    DrawString(280, STATUS_Y, '---', COL_TEXT, fb);
end;

procedure DrawTitle(fb: PByte);
begin
  DrawString(92, TITLE_Y, 'SUNLIGHT MINES', COL_ACCENT, fb);
end;

procedure DrawHint(fb: PByte);
begin
  DrawString(20, HINT_Y, 'L:REVEAL  R:FLAG  R:RESTART  ESC:EXIT', COL_TEXT, fb);
end;

procedure DrawFullUI(fb: PByte);
begin
  ClearScreen(COL_BG);
  FillRect(4, 2, 312, 22, COL_PANEL);
  FillRect(4, 24, 312, 26, COL_PANEL);
  DrawTitle(fb);
  DrawStatus(fb);
  DrawBoard(fb);
  DrawHint(fb);
  if GameWon then DrawBanner('FIELD CLEARED', True);
  if GameLost then DrawBanner('MINE TRIGGERED', False);
end;

procedure DrawBanner(msg: String; good: Boolean);
var
  col: Byte;
  len, sx: Integer;
begin
  col := COL_ACCENT;
  len := Length(msg);
  sx := (SCREEN_W - len * 6) div 2;
  FillRect(sx - 8, 90, len * 6 + 16, 20, COL_PANEL);
  DrawString(sx, 94, msg, col, Ptr(FB_SEG, 0));
end;

{ FPC TP mode lacks IntToStr in base, provide simple for our digits }
function IntToStr(v: Integer): String;
var
  s: String;
begin
  Str(v, s);
  IntToStr := s;
end;

end.
