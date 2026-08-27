typedef _Bool CK_Bool;
typedef struct { CK_Bool value; } CK_B1;
typedef struct { int value; } CK_I4;
typedef struct { long long a, b; } CK_I16;
typedef struct { long long a, b, c; } CK_I24;
typedef struct { double a; int b; } CK_DI;
typedef struct { double a, b, c, d; } CK_H4;

CK_Bool ck_bool(CK_Bool value) { return value; }
CK_I4 ck_i4(CK_I4 value) { return value; }
CK_I16 ck_i16(CK_I16 value) { return value; }
CK_I24 ck_i24(CK_I24 value) { return value; }
CK_DI ck_di(CK_DI value) { return value; }
CK_H4 ck_h4(CK_H4 value) { return value; }
int ck_checked(CK_I24 value, CK_I24 *result) {
  *result = value;
  return 0;
}
