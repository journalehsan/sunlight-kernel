unit Mouse33;
{$mode tp}
{$H-}
{$IMPLICITEXCEPTIONS OFF}

{ INT 33h polling support for Chronos. Edge detection done by caller. }

interface

type
  TMouseState = record
    X, Y: Integer;
    Buttons: Word;
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
  retAx, retBx: Word;
begin
  asm
    xor ax, ax
    int $33
    mov retAx, ax
    mov retBx, bx
  end;
  installed := (retAx = $FFFF);
  btnCount := retBx;
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
  mbx, mcx, mdx: Word;
begin
  asm
    mov ax, 3
    int $33
    mov mbx, bx
    mov mcx, cx
    mov mdx, dx
  end;
  st.Buttons := mbx;
  st.X := Integer(mcx);
  st.Y := Integer(mdx);
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
