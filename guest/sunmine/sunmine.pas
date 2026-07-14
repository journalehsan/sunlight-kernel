program SunlightMines;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ Sunlight Mines - 9x9 / 10 mines text-mode DOS app for Chronos.
  Uses direct B8000 writes (80x25 text mode 03h).
  Event-driven: no full-screen redraw per frame. }

uses
  DosApi, Video, Mouse33, Game, Draw, Storage;

var
  Installed: Boolean;
  BtnCount: Word;
  PrevButtons: Word;
  CurrButtons: Word;
  MouseSt: TMouseState;
  LastTimeSec: Word;
  TimerOriginSec: Word;
  Best: Word;
  CmdSeed: LongInt;
  Done: Boolean;
  RestartPending: Boolean;
  NeedsRedraw: Boolean;
  PrevWon, PrevLost: Boolean;

procedure UpdateTimer;
var
  t: TDosTime;
  sec: Word;
begin
  GetDosTime(t);
  sec := Word(t.Min) * 60 + t.Sec;
  LastTimeSec := sec;
  if TimerRunning then
  begin
    if TimerOriginSec = $FFFF then
      TimerOriginSec := sec;
    if sec >= TimerOriginSec then
      TickTimer(sec - TimerOriginSec)
    else
      TickTimer(sec + 3600 - TimerOriginSec);
  end
  else
    TimerOriginSec := $FFFF;
end;

procedure HandleLeftClick(cellX, cellY: Integer);
begin
  if (cellX < 0) or (cellX >= BOARD_W) or (cellY < 0) or (cellY >= BOARD_H) then
    Exit;
  if RevealCell(cellX, cellY) then
    { loss - reveal changed cells, will be redrawn }
  ;
  NeedsRedraw := True;
end;

procedure HandleRightClick(cellX, cellY: Integer);
begin
  if (cellX < 0) or (cellX >= BOARD_W) or (cellY < 0) or (cellY >= BOARD_H) then Exit;
  ToggleFlag(cellX, cellY);
  DrawCell(cellX, cellY);
  DrawStatus;
end;

function ScreenToCell(mx, my: Integer; var cx, cy: Integer): Boolean;
begin
  { mouse coords are in text columns/rows (0..79, 0..24) }
  cx := (mx - BOARD_COL) div CELL_W;
  cy := (my - BOARD_ROW);
  ScreenToCell := InBounds(cx, cy);
end;

procedure GameLoop;
var
  cx, cy: Integer;
  newLeft, newRight: Boolean;
  key: Char;
  tickCounter: Byte;
  timerDrawn: Word;
begin
  Done := False;
  tickCounter := 0;
  timerDrawn := 9999; { force initial status draw }

  while not Done do
  begin
    { cooperative yield only every N iterations to reduce trap overhead }
    if tickCounter >= 8 then
    begin
      asm
        int $28
      end;
      tickCounter := 0;
    end;
    Inc(tickCounter);

    NeedsRedraw := False;
    key := #0;

    { mouse }
    GetMouseState(MouseSt);
    CurrButtons := MouseSt.Buttons;
    newLeft := ((CurrButtons and 1) <> 0) and ((PrevButtons and 1) = 0);
    newRight := ((CurrButtons and 2) <> 0) and ((PrevButtons and 2) = 0);
    PrevButtons := CurrButtons;

    if newLeft then
    begin
      if ScreenToCell(MouseSt.X, MouseSt.Y, cx, cy) then
        HandleLeftClick(cx, cy);
    end;
    if newRight then
    begin
      if ScreenToCell(MouseSt.X, MouseSt.Y, cx, cy) then
        HandleRightClick(cx, cy);
    end;

    { keyboard - nonblocking check }
    asm
      mov ah, 1
      int $16
      jz @nokey
      mov ah, 0
      int $16
      mov key, al
      jmp @havekey
    @nokey:
      mov key, 0
    @havekey:
    end;

    if key <> #0 then
    begin
      case UpCase(key) of
        'R', 'N': begin
          RestartPending := True;
        end;
        #27: begin
          Done := True;
        end;
      end;
    end;

    { redraw after click reveals cells that may be many }
    if NeedsRedraw then
    begin
      if GameLost then
      begin
        DrawBoard;      { show all mines }
        DrawBanner('MINE TRIGGERED', ATTR_LOST);
      end
      else
      begin
        { only cells that changed are already revealed; draw board gives best result }
        DrawBoard;
      end;
      DrawStatus;
    end;

    UpdateTimer;

    { status timer update every ~2 real seconds, or when it changes }
    if (ElapsedSec <> timerDrawn) and TimerRunning then
    begin
      DrawStatus;
      timerDrawn := ElapsedSec;
    end;

    { restart }
    if RestartPending then
    begin
      RestartPending := False;
      Restart;
      Best := LoadBestTime;
      TimerOriginSec := $FFFF;
      timerDrawn := 9999;
      RedrawFull;
    end;

    { win detection }
    if GameWon and (not PrevWon) then
    begin
      Best := LoadBestTime;
      if (Best = 0) or (ElapsedSec < Best) then
      begin
        SaveBestTime(ElapsedSec);
        Best := ElapsedSec;
      end;
      DrawBoard;
      DrawStatus;
      DrawBanner('FIELD CLEARED', ATTR_WON);
    end;

    PrevWon := GameWon;
    PrevLost := GameLost;
  end;
end;

procedure Setup;
begin
  SetVideoMode03;
  HideTextCursor;
  ResetMouse(Installed, BtnCount);
  if Installed then
  begin
    SetMouseRange(0, 0, 79, 24);
    SetMousePos(40, 13);
    ShowMouse;
  end;

  if CmdSeed <> 0 then
    InitGame(CmdSeed)
  else
    InitGame(0);

  Best := LoadBestTime;
  PrevButtons := 0;
  LastTimeSec := 0;
  TimerOriginSec := $FFFF;
  RestartPending := False;
  PrevWon := False;
  PrevLost := False;

  RedrawFull;
end;

procedure Shutdown;
begin
  HideMouse;
  SetVideoMode03;
end;

begin
  CmdSeed := 0;
  Setup;
  GameLoop;
  Shutdown;
  DosExit(0);
end.
