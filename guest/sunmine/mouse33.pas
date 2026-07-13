unit Mouse33;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ INT 33h polling support for Chronos. Edge detection done by caller. }

interface

type
  TMouseState = record
    X, Y: Integer;
    Buttons: Word;  { bit0 left, bit1 right }
  end;

procedure ResetMouse(var installed: Boolean; var btnCount: Word);
procedure ShowMouse;
procedure HideMouse;
procedure GetMouseState(var st: TMouseState);
procedure SetMouseRange(minX, minY, maxX, maxY: Integer);
procedure SetMousePos(x, y: Integer);

implementation

procedure ResetMouse(var installed: Boolean; var btnCount: Word);
var
  ax, bx: Word;
begin
  ax := 0;
  bx := 0;
  asm
    mov ax, 0
    int $33
    mov axResult, ax
    mov bxResult, bx
  end;
  installed := (ax = $FFFF);
  btnCount := bx;
end;

procedure ShowMouse;
begin
  asm
    mov ax, 1
    int $33
  end;
end;

procedure HideMouse;
begin
  asm
    mov ax, 2
    int $33
  end;
end;

procedure GetMouseState(var st: TMouseState);
var
  bx, cx, dx: Word;
begin
  asm
    mov ax, 3
    int $33
    mov bxResult, bx
    mov cxResult, cx
    mov dxResult, dx
  end;
  st.Buttons := bx;
  st.X := Integer(cx);
  st.Y := Integer(dx);
end;

procedure SetMouseRange(minX, minY, maxX, maxY: Integer);
begin
  asm
    mov ax, 7
    mov cx, minX
    mov dx, maxX
    int $33
    mov ax, 8
    mov cx, minY
    mov dx, maxY
    int $33
  end;
end;

procedure SetMousePos(x, y: Integer);
begin
  asm
    mov ax, 4
    mov cx, x
    mov dx, y
    int $33
  end;
end;

end.
