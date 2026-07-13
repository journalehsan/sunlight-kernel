program SunlightDosShell;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

uses Dos;

const
  MaxLine = 126;
  MaxBatchDepth = 8;

var
  EchoEnabled: Boolean;
  PromptText: String;
  LastError: Integer;
  ExitShell: Boolean;
  ExitCode: Integer;

procedure PrintError(const Prefix: String; const Code: Integer);
begin
  if Prefix <> '' then
    Write(Prefix, ': ');
  case Code of
    2: WriteLn('File not found');
    3: WriteLn('Path not found');
    5: WriteLn('Access denied');
    6: WriteLn('Invalid handle');
    8: WriteLn('Not enough memory');
    15: WriteLn('Invalid drive');
  else
    WriteLn('DOS error ', Code);
  end;
end;

procedure ClearScreen;
begin
  asm
    mov ax, $0003
    int $10
  end;
end;

procedure WritePrompt;
begin
  Write('CMD C:\>');
end;

function TrimLeft(const Value: String): String;
var
  Index: Integer;
begin
  Index := 1;
  while (Index <= Length(Value)) and (Value[Index] <= ' ') do
    Inc(Index);
  TrimLeft := Copy(Value, Index, Length(Value));
end;

function Upper(const Value: String): String;
var
  Index: Integer;
  ResultValue: String;
begin
  ResultValue := Value;
  for Index := 1 to Length(ResultValue) do
    ResultValue[Index] := UpCase(ResultValue[Index]);
  Upper := ResultValue;
end;

procedure SplitCommand(const Line: String; var Name, Args: String);
var
  Index: Integer;
  Quoted: Boolean;
begin
  Name := '';
  Args := TrimLeft(Line);
  Index := 1;
  Quoted := False;
  while Index <= Length(Args) do
  begin
    if Args[Index] = '"' then
      Quoted := not Quoted
    else if (Args[Index] <= ' ') and not Quoted then
      Break;
    Inc(Index);
  end;
  Name := Upper(Copy(Args, 1, Index - 1));
  Args := TrimLeft(Copy(Args, Index, Length(Args)));
end;

procedure CommandDir(const Pattern: String);
var
  Search: SearchRec;
  Count: Integer;
  Files: LongInt;
  Match: String;
begin
  Match := Pattern;
  if Match = '' then
    Match := '*.*';
  FindFirst(Match, AnyFile, Search);
  Count := 0;
  Files := 0;
  while DosError = 0 do
  begin
    if (Search.Attr and Directory) <> 0 then
      Write('<DIR> ')
    else
    begin
      Write('      ');
      Files := Files + Search.Size;
    end;
    WriteLn(Search.Name, ' ', Search.Size);
    Inc(Count);
    FindNext(Search);
  end;
  WriteLn(Count, ' item(s) ', Files, ' bytes');
end;

procedure CommandType(const FileName: String);
var
  Input: Text;
  Line: String;
begin
  Assign(Input, FileName);
  {$I-} Reset(Input); {$I+}
  if IOResult <> 0 then
  begin
    PrintError('', 2);
    Exit;
  end;
  while not Eof(Input) do
  begin
    ReadLn(Input, Line);
    WriteLn(Line);
  end;
  Close(Input);
end;

procedure CommandCopy(const Args: String);
var
  SourceName, DestinationName: String;
  Index: Integer;
  SourceFile, DestinationFile: File;
  Buffer: array[1..512] of Byte;
  ReadCount, WriteCount: Word;
  Total: LongInt;
begin
  Index := Pos(' ', Args);
  if Index = 0 then
  begin
    WriteLn('Usage: COPY source destination');
    Exit;
  end;
  SourceName := TrimLeft(Copy(Args, 1, Index - 1));
  DestinationName := TrimLeft(Copy(Args, Index + 1, Length(Args)));
  if Upper(SourceName) = Upper(DestinationName) then
  begin
    WriteLn('Cannot copy a file onto itself.');
    Exit;
  end;
  Assign(SourceFile, SourceName);
  {$I-} Reset(SourceFile, 1); {$I+}
  if IOResult <> 0 then
  begin
    PrintError('', 2);
    Exit;
  end;
  Assign(DestinationFile, DestinationName);
  {$I-} Rewrite(DestinationFile, 1); {$I+}
  if IOResult <> 0 then
  begin
    Close(SourceFile);
    PrintError('', 5);
    Exit;
  end;
  Total := 0;
  repeat
    BlockRead(SourceFile, Buffer, SizeOf(Buffer), ReadCount);
    if ReadCount <> 0 then
    begin
      BlockWrite(DestinationFile, Buffer, ReadCount, WriteCount);
      Total := Total + WriteCount;
    end;
  until ReadCount = 0;
  Close(SourceFile);
  Close(DestinationFile);
  WriteLn(Total, ' byte(s) copied.');
end;

procedure ExecuteLine(const InputLine: String; BatchDepth: Integer); forward;

procedure RunBatch(const FileName: String; BatchDepth: Integer);
var
  Input: Text;
  Line: String;
begin
  if BatchDepth >= MaxBatchDepth then
  begin
    WriteLn('Batch nesting limit reached.');
    LastError := 1;
    Exit;
  end;
  Assign(Input, FileName);
  {$I-} Reset(Input); {$I+}
  if IOResult <> 0 then
  begin
    PrintError('', 2);
    LastError := 1;
    Exit;
  end;
  while not Eof(Input) and not ExitShell do
  begin
    ReadLn(Input, Line);
    ExecuteLine(Line, BatchDepth + 1);
  end;
  Close(Input);
end;

procedure ExecuteExternal(const Name, Args: String; BatchDepth: Integer);
var
  Candidate: String;
begin
  Candidate := Name;
  if Pos('.', Candidate) = 0 then
  begin
    if FSearch(Candidate + '.COM', GetEnv('PATH')) <> '' then
      Candidate := FSearch(Candidate + '.COM', GetEnv('PATH'))
    else if FSearch(Candidate + '.EXE', GetEnv('PATH')) <> '' then
      Candidate := FSearch(Candidate + '.EXE', GetEnv('PATH'))
    else if FSearch(Candidate + '.BAT', GetEnv('PATH')) <> '' then
      Candidate := FSearch(Candidate + '.BAT', GetEnv('PATH'));
  end;
  if Upper(Copy(Candidate, Length(Candidate) - 3, 4)) = '.BAT' then
  begin
    RunBatch(Candidate, BatchDepth);
    Exit;
  end;
  SwapVectors;
  Exec(Candidate, Args);
  SwapVectors;
  LastError := DosExitCode;
  if DosError <> 0 then
  begin
    PrintError('Bad command or executable', DosError);
    LastError := 1;
  end;
end;

procedure ExecuteLine(const InputLine: String; BatchDepth: Integer);
var
  Line, Name, Args: String;
  Code: Integer;
  DeleteFile: File;
begin
  Line := TrimLeft(InputLine);
  if Line = '' then
    Exit;
  if Line[1] = '@' then
    Line := TrimLeft(Copy(Line, 2, Length(Line)));
  if (Line = '') or (Line[1] = ':') then
    Exit;
  SplitCommand(Line, Name, Args);
  LastError := 0;
  if (Name = 'REM') then
    Exit
  else if (Name = 'CLS') then
    ClearScreen
  else if (Name = 'ECHO') then
  begin
    if Upper(Args) = 'ON' then EchoEnabled := True
    else if Upper(Args) = 'OFF' then EchoEnabled := False
    else if Args = '.' then WriteLn
    else WriteLn(Args);
  end
  else if (Name = 'DIR') then CommandDir(Args)
  else if (Name = 'TYPE') then CommandType(Args)
  else if (Name = 'COPY') then CommandCopy(Args)
  else if (Name = 'DEL') or (Name = 'ERASE') then
  begin
    Assign(DeleteFile, Args);
    {$I-} Erase(DeleteFile); {$I+}
    Code := IOResult;
    if Code <> 0 then PrintError('', Code);
    LastError := Code;
  end
  else if (Name = 'MD') or (Name = 'MKDIR') then
  begin
    {$I-} MkDir(Args); {$I+}
    LastError := IOResult;
    if LastError <> 0 then PrintError('', LastError);
  end
  else if (Name = 'RD') or (Name = 'RMDIR') then
  begin
    {$I-} RmDir(Args); {$I+}
    LastError := IOResult;
    if LastError <> 0 then PrintError('', LastError);
  end
  else if (Name = 'CD') or (Name = 'CHDIR') then
  begin
    if Args = '' then
      WriteLn(GetEnv('CD'))
    else
    begin
      {$I-} ChDir(Args); {$I+}
      LastError := IOResult;
      if LastError <> 0 then PrintError('', LastError);
    end;
  end
  else if (Name = 'VER') then
  begin
    WriteLn('Sunlight DOS Shell 0.1');
    WriteLn('Chronos DOS compatibility runtime');
  end
  else if (Name = 'VOL') then
    WriteLn('Volume in drive C is SUNLIGHT')
  else if (Name = 'HELP') then
    WriteLn('CLS CD DIR ECHO TYPE COPY DEL MD RD VER VOL PAUSE EXIT HELP')
  else if (Name = 'PAUSE') then
  begin
    Write('Press any key to continue . . .');
    ReadLn;
  end
  else if (Name = 'EXIT') then
  begin
    if Args <> '' then Val(Args, ExitCode, Code) else ExitCode := LastError;
    ExitShell := True;
  end
  else
    ExecuteExternal(Name, Args, BatchDepth);
end;

procedure PrintBanner;
begin
  WriteLn('Sunlight DOS Shell 0.1');
  WriteLn('Chronos 16-bit real-mode environment');
  WriteLn('Type HELP for available commands.');
  WriteLn;
end;

var
  Line: String;
  Index: Integer;
  Command: String;
begin
  EchoEnabled := True;
  LastError := 0;
  ExitShell := False;
  ExitCode := 0;
  PromptText := '$P$G';
  if ParamCount >= 2 then
  begin
    if Upper(ParamStr(1)) = '/C' then
    begin
      Command := ParamStr(2);
      if Upper(Copy(Command, Length(Command) - 3, 4)) = '.BAT' then
        RunBatch(Command, 0)
      else
        ExecuteLine(Command, 0);
      if ExitShell then
        Halt(ExitCode)
      else
        Halt(LastError);
    end;
  end;
  PrintBanner;
  while not ExitShell do
  begin
    WritePrompt;
    ReadLn(Line);
    ExecuteLine(Line, 0);
  end;
  Halt(ExitCode);
end.
