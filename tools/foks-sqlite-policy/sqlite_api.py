"""Small diagnostic-only binding to a separately built, checksum-pinned SQLite.

No existing database paths, extension loading, or production application API.
"""
import ctypes as C

AUTHORIZE = C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_int, C.c_char_p, C.c_char_p, C.c_char_p, C.c_char_p)
FUNCTION = C.CFUNCTYPE(None, C.c_void_p, C.c_int, C.c_void_p)


class SqlError(RuntimeError):
    def __init__(self, code, message):
        super().__init__(message[:240])
        self.code = code


class Engine:
    def __init__(self, path):
        self.lib = C.CDLL(str(path))
        signatures = {
            'open': ([C.c_char_p, C.POINTER(C.c_void_p)], C.c_int),
            'close': ([C.c_void_p], C.c_int),
            'errmsg': ([C.c_void_p], C.c_char_p),
            'extended_errcode': ([C.c_void_p], C.c_int),
            'prepare_v2': ([C.c_void_p, C.c_char_p, C.c_int, C.POINTER(C.c_void_p), C.POINTER(C.c_char_p)], C.c_int),
            'finalize': ([C.c_void_p], C.c_int),
            'step': ([C.c_void_p], C.c_int),
            'column_count': ([C.c_void_p], C.c_int),
            'column_type': ([C.c_void_p, C.c_int], C.c_int),
            'column_int64': ([C.c_void_p, C.c_int], C.c_int64),
            'column_text': ([C.c_void_p, C.c_int], C.c_void_p),
            'column_bytes': ([C.c_void_p, C.c_int], C.c_int),
            'bind_parameter_count': ([C.c_void_p], C.c_int),
            'bind_null': ([C.c_void_p, C.c_int], C.c_int),
            'set_authorizer': ([C.c_void_p, AUTHORIZE, C.c_void_p], C.c_int),
            'libversion': ([], C.c_char_p),
            'sourceid': ([], C.c_char_p),
            'limit': ([C.c_void_p, C.c_int, C.c_int], C.c_int),
            'result_int': ([C.c_void_p, C.c_int], None),
            'create_function_v2': ([C.c_void_p, C.c_char_p, C.c_int, C.c_int, C.c_void_p, FUNCTION, C.c_void_p, C.c_void_p, C.c_void_p], C.c_int),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.lib, 'sqlite3_' + name)
            function.argtypes, function.restype = arguments, result
            setattr(self, name, function)

    def connect(self):
        return Database(self)


class Database:
    def __init__(self, engine):
        self.engine = engine
        self.handle = C.c_void_p()
        self.functions = set()
        self.callback_error = False
        self.statements = 0
        self.last_parameter_count = 0
        self.test_functions = []
        status = engine.open(b':memory:', C.byref(self.handle))
        if status:
            if self.handle:
                engine.close(self.handle)
            self.handle = C.c_void_p()
            raise SqlError(status, 'cannot open diagnostic memory database')

        def authorize(_context, action, _arg1, arg2, _db, _accessor):
            try:
                if action == 31:  # SQLITE_FUNCTION
                    name = (arg2 or b'').decode('utf-8', errors='strict')
                    if len(name) > 128 or len(self.functions) >= 256:
                        self.callback_error = True
                        return 1
                    self.functions.add(name.lower())
                return 0
            except BaseException:
                self.callback_error = True
                return 1
        self.authorizer = AUTHORIZE(authorize)
        try:
            self.check(engine.set_authorizer(self.handle, self.authorizer, None))
        except BaseException:
            self.close()
            raise

    def check(self, code):
        if code:
            raise SqlError(self.engine.extended_errcode(self.handle), self.engine.errmsg(self.handle).decode('utf-8', errors='replace'))

    def query(self, sql, maximum_rows=25000, maximum_bytes=8 * 1024 * 1024):
        if not self.handle or not isinstance(sql, str) or '\0' in sql or len(sql.encode()) > 100000:
            raise ValueError('diagnostic statement bound')
        encoded = sql.encode('utf-8')
        statement, tail = C.c_void_p(), C.c_char_p()
        size = 0
        rows = []
        try:
            self.check(self.engine.prepare_v2(self.handle, encoded, len(encoded) + 1, C.byref(statement), C.byref(tail)))
            if not statement or (tail.value or b'').strip():
                raise ValueError('expected exactly one diagnostic statement')
            self.statements += 1
            self.last_parameter_count = self.engine.bind_parameter_count(statement)
            if self.last_parameter_count > 100:
                raise ValueError('diagnostic parameter bound')
            for index in range(1, self.last_parameter_count + 1):
                self.check(self.engine.bind_null(statement, index))
            columns = self.engine.column_count(statement)
            if columns > 128:
                raise ValueError('diagnostic column bound')
            while True:
                status = self.engine.step(statement)
                if status == 101:  # SQLITE_DONE
                    break
                if status != 100:
                    self.check(status)
                if len(rows) >= maximum_rows:
                    raise ValueError('diagnostic row bound')
                row = []
                for index in range(columns):
                    kind = self.engine.column_type(statement, index)
                    if kind == 5:
                        value = None
                        size += 1
                    elif kind == 1:
                        value = self.engine.column_int64(statement, index)
                        size += 20
                    elif kind == 3:
                        pointer = self.engine.column_text(statement, index)
                        length = self.engine.column_bytes(statement, index)
                        if length < 0 or length > 65536 or (length and not pointer):
                            raise ValueError('diagnostic text bound')
                        size += length
                        if size > maximum_bytes:
                            raise ValueError('diagnostic aggregate bound')
                        value = C.string_at(pointer, length).decode('utf-8', errors='strict')
                    else:
                        raise ValueError('diagnostic unexpected result type')
                    if size > maximum_bytes:
                        raise ValueError('diagnostic aggregate bound')
                    row.append(value)
                rows.append(row)
            if self.callback_error:
                raise ValueError('diagnostic authorizer callback failed')
            return rows
        finally:
            if statement:
                self.engine.finalize(statement)

    def register_test_function(self, innocuous):
        # Only the synthetic dependency-completeness tests call this method.
        # Registration occurs on a separate database, never the trusted profile.
        def function(context, _argc, _argv):
            self.engine.result_int(context, 7)
        callback = FUNCTION(function)
        self.test_functions.append(callback)
        flags = 1 | 0x800 | (0x200000 if innocuous else 0)  # UTF8, deterministic, innocuous
        self.check(self.engine.create_function_v2(self.handle, b'policy_test', 0, flags, None, callback, None, None, None))

    def close(self):
        if self.handle:
            code = self.engine.close(self.handle)
            if code:
                raise SqlError(code, 'diagnostic connection retained a statement')
            self.handle = C.c_void_p()
            self.test_functions.clear()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
