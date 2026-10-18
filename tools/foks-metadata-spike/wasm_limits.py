"""Read the declared Wasm linear-memory limit; do not allocate/grow memory."""
def memory_limits(data):
    if data[:8] != b'\x00asm\x01\x00\x00\x00':
        raise ValueError('wasm header')
    def uleb(offset):
        value=0
        for shift in range(0,35,7):
            if offset>=len(data):raise ValueError('short LEB')
            byte=data[offset];offset+=1;value|=(byte&127)<<shift
            if byte<128:return value,offset
        raise ValueError('long LEB')
    offset=8;found=None
    while offset<len(data):
        kind=data[offset];size,start=uleb(offset+1);end=start+size
        if end>len(data):raise ValueError('section bounds')
        if kind==5:
            if found is not None:raise ValueError('duplicate memory section')
            count,cursor=uleb(start)
            if count!=1:raise ValueError('expected one memory')
            flags,cursor=uleb(cursor)
            if flags!=1:raise ValueError('memory requires unshared 32-bit maximum')
            minimum,cursor=uleb(cursor);maximum,cursor=uleb(cursor)
            if cursor!=end or minimum>maximum or maximum>1024:raise ValueError('memory exceeds 64 MiB')
            found=(minimum,maximum)
        offset=end
    if found is None:raise ValueError('missing memory declaration')
    return found
