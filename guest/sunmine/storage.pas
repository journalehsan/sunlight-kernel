unit Storage;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ Persistent best time in C:\STATE\MINEBEST.DAT
  Format: 'SMIN' (4) + version (1) + best_seconds (2 LE) }

interface

const
  STATE_DIR = 'C:\STATE';
  BEST_FILE = 'C:\STATE\MINEBEST.DAT';
  BEST_MAGIC: array[0..3] of Char = 'SMIN';
  BEST_VERSION = 1;

function LoadBestTime: Word;
procedure SaveBestTime(seconds: Word);

implementation

uses DosApi;

function LoadBestTime: Word;
var
  h: Word;
  buf: array[0..6] of Byte;
  n: Word;
  sec: Word;
begin
  LoadBestTime := 0;
  h := DosOpenFile(BEST_FILE, False);
  if h = 0 then Exit;
  n := DosRead(h, @buf[0], 7);
  DosClose(h);
  if (n = 7) and
     (buf[0] = Ord('S')) and (buf[1] = Ord('M')) and
     (buf[2] = Ord('I')) and (buf[3] = Ord('N')) then
  begin
    if buf[4] = BEST_VERSION then
    begin
      sec := buf[5] + (buf[6] shl 8);
      LoadBestTime := sec;
    end;
  end;
end;

procedure SaveBestTime(seconds: Word);
var
  h: Word;
  buf: array[0..6] of Byte;
begin
  DosMkdir(STATE_DIR);
  h := DosCreateFile(BEST_FILE);
  if h = 0 then Exit;
  buf[0] := Ord('S'); buf[1] := Ord('M'); buf[2] := Ord('I'); buf[3] := Ord('N');
  buf[4] := BEST_VERSION;
  buf[5] := Byte(seconds and $FF);
  buf[6] := Byte((seconds shr 8) and $FF);
  DosWrite(h, @buf[0], 7);
  DosClose(h);
end;

end.
