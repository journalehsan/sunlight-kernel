unit Game;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ 9x9 beginner Minesweeper core. No recursion. Deterministic with seed. }

interface

const
  BOARD_W = 9;
  BOARD_H = 9;
  MINE_COUNT = 10;

type
  TCell = record
    IsMine: Boolean;
    IsRevealed: Boolean;
    IsFlagged: Boolean;
    Neighbor: Byte;
  end;

  TBoard = array[0..BOARD_H-1, 0..BOARD_W-1] of TCell;

  TPos = record X, Y: Byte; end;

var
  Board: TBoard;
  MinesPlaced: Boolean;
  GameWon: Boolean;
  GameLost: Boolean;
  FirstClick: Boolean;
  RemainingMines: Integer;
  ElapsedSec: Word;
  TimerRunning: Boolean;
  Seed: LongInt;

procedure InitGame(useSeed: LongInt);
procedure PlaceMinesSafe(firstX, firstY: Integer);
function RevealCell(x, y: Integer): Boolean; { returns true if mine hit }
procedure ToggleFlag(x, y: Integer);
function CheckWin: Boolean;
function InBounds(x, y: Integer): Boolean;
procedure Restart;
procedure StopTimer;
procedure TickTimer(currentSec: Word);

{ queue for flood }
const
  MAX_QUEUE = BOARD_W * BOARD_H;

implementation

var
  Queue: array[0..MAX_QUEUE-1] of TPos;
  QHead, QTail: Integer;

procedure QInit;
begin QHead := 0; QTail := 0; end;

procedure QEnq(x, y: Byte);
begin
  if QTail < MAX_QUEUE then
  begin
    Queue[QTail].X := x;
    Queue[QTail].Y := y;
    Inc(QTail);
  end;
end;

function QDeq(var x, y: Byte): Boolean;
begin
  if QHead = QTail then begin QDeq := False; Exit; end;
  x := Queue[QHead].X;
  y := Queue[QHead].Y;
  Inc(QHead);
  QDeq := True;
end;

function InBounds(x, y: Integer): Boolean;
begin
  InBounds := (x >= 0) and (x < BOARD_W) and (y >= 0) and (y < BOARD_H);
end;

procedure InitBoard;
var
  y, x: Integer;
begin
  for y := 0 to BOARD_H-1 do
    for x := 0 to BOARD_W-1 do
    begin
      Board[y,x].IsMine := False;
      Board[y,x].IsRevealed := False;
      Board[y,x].IsFlagged := False;
      Board[y,x].Neighbor := 0;
    end;
  MinesPlaced := False;
  GameWon := False;
  GameLost := False;
  FirstClick := True;
  RemainingMines := MINE_COUNT;
  ElapsedSec := 0;
  TimerRunning := False;
end;

{ Simple 16-bit LCG RNG suitable for i8086 }
function NextRand: Word;
begin
  Seed := (Seed * 1103515245 + 12345) and $7FFFFFFF;
  NextRand := Word(Seed shr 16);
end;

procedure InitGame(useSeed: LongInt);
begin
  if useSeed <> 0 then Seed := useSeed else Seed := $12345678;
  InitBoard;
end;

procedure PlaceMinesSafe(firstX, firstY: Integer);
var
  placed, attempts, rx, ry, dx, dy, nx, ny: Integer;
  ok: Boolean;
begin
  if MinesPlaced then Exit;
  placed := 0;
  attempts := 0;
  while (placed < MINE_COUNT) and (attempts < 2000) do
  begin
    Inc(attempts);
    rx := NextRand mod BOARD_W;
    ry := NextRand mod BOARD_H;
    ok := True;
    { exclude click and neighbors for first click safety }
    for dy := -1 to 1 do
      for dx := -1 to 1 do
      begin
        nx := firstX + dx;
        ny := firstY + dy;
        if (rx = nx) and (ry = ny) then ok := False;
      end;
    if not ok then Continue;
    if not Board[ry, rx].IsMine then
    begin
      Board[ry, rx].IsMine := True;
      Inc(placed);
    end;
  end;
  { If still short (rare), fill remaining without exclusion to guarantee count }
  while placed < MINE_COUNT do
  begin
    rx := NextRand mod BOARD_W;
    ry := NextRand mod BOARD_H;
    if not Board[ry, rx].IsMine then
    begin
      Board[ry, rx].IsMine := True;
      Inc(placed);
    end;
  end;

  { Compute neighbor counts }
  for ry := 0 to BOARD_H-1 do
    for rx := 0 to BOARD_W-1 do
    begin
      if Board[ry, rx].IsMine then Continue;
      placed := 0;
      for dy := -1 to 1 do
        for dx := -1 to 1 do
        begin
          nx := rx + dx; ny := ry + dy;
          if InBounds(nx, ny) and Board[ny, nx].IsMine then Inc(placed);
        end;
      Board[ry, rx].Neighbor := placed;
    end;
  MinesPlaced := True;
  TimerRunning := True;
  FirstClick := False;
end;

procedure RevealRegion(x, y: Integer);
var
  cx, cy: Byte;
  nx, ny, d: Integer;
begin
  if not InBounds(x, y) then Exit;
  if Board[y, x].IsRevealed or Board[y, x].IsFlagged then Exit;
  Board[y, x].IsRevealed := True;
  if Board[y, x].Neighbor > 0 then Exit;
  { iterative flood using queue }
  QInit;
  QEnq(Byte(x), Byte(y));
  while QDeq(cx, cy) do
  begin
    for d := 0 to 7 do
    begin
      case d of
        0: begin nx:=cx-1; ny:=cy-1; end;
        1: begin nx:=cx;   ny:=cy-1; end;
        2: begin nx:=cx+1; ny:=cy-1; end;
        3: begin nx:=cx-1; ny:=cy;   end;
        4: begin nx:=cx+1; ny:=cy;   end;
        5: begin nx:=cx-1; ny:=cy+1; end;
        6: begin nx:=cx;   ny:=cy+1; end;
        7: begin nx:=cx+1; ny:=cy+1; end;
      end;
      if InBounds(nx, ny) and not Board[ny, nx].IsRevealed and not Board[ny, nx].IsFlagged then
      begin
        Board[ny, nx].IsRevealed := True;
        if Board[ny, nx].Neighbor = 0 then
          QEnq(Byte(nx), Byte(ny));
      end;
    end;
  end;
end;

function RevealCell(x, y: Integer): Boolean;
begin
  RevealCell := False;
  if not InBounds(x, y) or GameLost or GameWon then Exit;
  if Board[y, x].IsRevealed then Exit;
  if Board[y, x].IsFlagged then Exit;
  if FirstClick then
  begin
    PlaceMinesSafe(x, y);
  end;
  if Board[y, x].IsMine then
  begin
    GameLost := True;
    TimerRunning := False;
    Board[y, x].IsRevealed := True;
    RevealCell := True;
    Exit;
  end;
  RevealRegion(x, y);
  if CheckWin then
  begin
    GameWon := True;
    TimerRunning := False;
  end;
end;

procedure ToggleFlag(x, y: Integer);
begin
  if not InBounds(x, y) or GameLost or GameWon then Exit;
  if Board[y, x].IsRevealed then Exit;
  if Board[y, x].IsFlagged then
  begin
    Board[y, x].IsFlagged := False;
    Inc(RemainingMines);
  end
  else
  begin
    Board[y, x].IsFlagged := True;
    Dec(RemainingMines);
  end;
end;

function CheckWin: Boolean;
var
  x, y: Integer;
begin
  for y := 0 to BOARD_H-1 do
    for x := 0 to BOARD_W-1 do
      if (not Board[y,x].IsMine) and (not Board[y,x].IsRevealed) then
      begin
        CheckWin := False;
        Exit;
      end;
  CheckWin := True;
end;

procedure Restart;
begin
  InitBoard;
  { next click will place with new or same seed behavior via caller }
end;

procedure StopTimer;
begin
  TimerRunning := False;
end;

procedure TickTimer(currentSec: Word);
begin
  if TimerRunning then
  begin
    if ElapsedSec < currentSec then
      ElapsedSec := currentSec;
    if ElapsedSec > 999 then ElapsedSec := 999;
  end;
end;

end.
