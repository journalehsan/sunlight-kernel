unit Draw;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ Text-mode drawing for Sunlight Mines using direct B8000 writes.
  Board is 9x9, each cell occupies 2 B8000 columns. }

interface

uses Video, Game, Storage;

const
  { board positioning in text columns/rows }
  BOARD_ROW = 3;
  BOARD_COL = 31;   { 9 cells * 2 cols = 18; (80-18)/2 = 31 }
  CELL_W    = 2;

  { UI rows }
  TITLE_ROW  = 0;
  STATUS_ROW = 1;
  HINT_ROW   = 13;
  BANNER_ROW = 15;

procedure DrawTitle;
procedure DrawStatus;
procedure DrawHint;
procedure DrawBanner(msg: String; attr: Byte);
procedure DrawBoard;
procedure DrawCell(x, y: Integer);
procedure RedrawFull;

implementation

function NumAttr(n: Integer): Byte;
begin
  case n of
    1: NumAttr := ATTR_NUM1;
    2: NumAttr := ATTR_NUM2;
    3: NumAttr := ATTR_NUM3;
    4: NumAttr := ATTR_NUM4;
    5: NumAttr := ATTR_NUM5;
    6: NumAttr := ATTR_NUM6;
    7: NumAttr := ATTR_NUM7;
    8: NumAttr := ATTR_NUM8;
  else
    NumAttr := ATTR_TEXT;
  end;
end;

{ Convert game x,y to screen column/row }
procedure CellPos(x, y: Integer; var scrRow, scrCol: Integer);
begin
  scrRow := BOARD_ROW + y;
  scrCol := BOARD_COL + x * CELL_W;
end;

procedure DrawTitle;
begin
  PutText(TITLE_ROW, 29, 'SUNLIGHT MINES', ATTR_TITLE);
end;

procedure DrawStatus;
var
  s: String;
  best: Word;
begin
  FillRow(STATUS_ROW, 0, SCREEN_COLS - 1, ' ', ATTR_TEXT);
  PutText(STATUS_ROW, 4, 'MINES', ATTR_STATUS);
  if RemainingMines < 0 then
    PutText(STATUS_ROW, 10, '00', ATTR_TEXT)
  else if RemainingMines < 10 then
  begin
    s := '0';
    s := s + Chr(Ord('0') + RemainingMines);
    PutText(STATUS_ROW, 10, s, ATTR_TEXT);
  end
  else
  begin
    s := Chr(Ord('0') + (RemainingMines div 10));
    s := s + Chr(Ord('0') + (RemainingMines mod 10));
    PutText(STATUS_ROW, 10, s, ATTR_TEXT);
  end;
  PutText(STATUS_ROW, 26, 'TIME', ATTR_STATUS);
  if ElapsedSec > 999 then
    PutText(STATUS_ROW, 31, '999', ATTR_TEXT)
  else
  begin
    s := Chr(Ord('0') + (ElapsedSec div 100));
    s := s + Chr(Ord('0') + ((ElapsedSec div 10) mod 10));
    s := s + Chr(Ord('0') + (ElapsedSec mod 10));
    PutText(STATUS_ROW, 31, s, ATTR_TEXT);
  end;
  PutText(STATUS_ROW, 50, 'BEST', ATTR_STATUS);
  best := LoadBestTime;
  if best > 0 then
  begin
    s := Chr(Ord('0') + (best div 100));
    s := s + Chr(Ord('0') + ((best div 10) mod 10));
    s := s + Chr(Ord('0') + (best mod 10));
    PutText(STATUS_ROW, 55, s, ATTR_TEXT);
  end
  else
    PutText(STATUS_ROW, 55, '---', ATTR_TEXT);
end;

procedure DrawHint;
begin
  FillRow(HINT_ROW, 0, SCREEN_COLS - 1, ' ', ATTR_TEXT);
  PutText(HINT_ROW, 8, 'Left=Reveal  Right=Flag  R=Restart  Esc=Exit', ATTR_TEXT);
end;

procedure DrawBanner(msg: String; attr: Byte);
var
  col, i: Integer;
begin
  col := (SCREEN_COLS - Length(msg)) div 2;
  { clear banner row }
  FillRow(BANNER_ROW, 0, SCREEN_COLS - 1, ' ', ATTR_TEXT);
  FillRow(BANNER_ROW + 1, 0, SCREEN_COLS - 1, ' ', ATTR_TEXT);
  PutText(BANNER_ROW, col, msg, attr);
end;

{ Draw a single game cell at board position x,y using current Board state }
procedure DrawCell(x, y: Integer);
var
  sr, sc: Integer;
  cell: TCell;
  leftChar, rightChar: Char;
  leftAttr, rightAttr: Byte;
begin
  if not InBounds(x, y) then Exit;
  CellPos(x, y, sr, sc);
  cell := Board[y, x];

  if cell.IsRevealed then
  begin
    if cell.IsMine then
    begin
      leftChar := '*'; leftAttr := ATTR_MINE;
      rightChar := ' '; rightAttr := ATTR_REVEALED;
    end
    else if cell.Neighbor > 0 then
    begin
      leftChar := ' ';
      rightChar := Chr(Ord('0') + cell.Neighbor);
      leftAttr  := ATTR_REVEALED;
      rightAttr := NumAttr(cell.Neighbor);
    end
    else
    begin
      leftChar := ' '; leftAttr  := ATTR_REVEALED;
      rightChar := ' '; rightAttr := ATTR_REVEALED;
    end;
  end
  else if cell.IsFlagged then
  begin
    if GameLost and (not cell.IsMine) then
    begin
      leftChar := 'X'; leftAttr := ATTR_FLAG_WRONG;
      rightChar:= #219; rightAttr:= ATTR_FLAG_WRONG;
    end
    else
    begin
      leftChar := 'F'; leftAttr  := ATTR_FLAG;
      rightChar:= #219; rightAttr:= ATTR_HIDDEN;
    end;
  end
  else
  begin
    { unrevealed, show mines if lost }
    if GameLost and cell.IsMine then
    begin
      leftChar := '*'; leftAttr  := ATTR_MINE_BG;
      rightChar:= ' '; rightAttr := ATTR_MINE_BG;
    end
    else
    begin
      leftChar := #219; leftAttr := ATTR_HIDDEN;
      rightChar:= #219; rightAttr:= ATTR_HIDDEN;
    end;
  end;

  PutCell(sr, sc,   leftChar,  leftAttr);
  PutCell(sr, sc+1, rightChar, rightAttr);
end;

procedure DrawBoard;
var
  cx, cy: Integer;
begin
  for cy := 0 to BOARD_H - 1 do
    for cx := 0 to BOARD_W - 1 do
      DrawCell(cx, cy);
end;

procedure RedrawFull;
begin
  ClearScreen;
  DrawTitle;
  DrawStatus;
  DrawBoard;
  DrawHint;
  if GameWon then
    DrawBanner('FIELD CLEARED', ATTR_WON)
  else if GameLost then
    DrawBanner('MINE TRIGGERED', ATTR_LOST);
end;

end.
