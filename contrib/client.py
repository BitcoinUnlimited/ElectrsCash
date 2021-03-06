import json
import socket

class Client:
    def __init__(self, addr):
        self.s = socket.create_connection(addr)
        self.f = self.s.makefile('r')
        self.id = 0

    def call(self, method, *args):
        req = {
            'id': self.id,
            'method': method,
            'params': list(args),
        }
        msg = json.dumps(req) + '\n'
        self.s.sendall(msg.encode('ascii'))
        return json.loads(self.f.readline())

def guess_type(arg):
    if arg.lower() == "true":
        return True
    if arg.lower() == "false":
        return False
    if arg.isdigit():
        return int(arg)
    try:
        return float(arg)
    except Exception as _:
        pass
    return arg

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--port', action='store')
    parser.add_argument('--server', action='store')
    parser.add_argument('method')
    parser.add_argument('args', nargs='*')
    args = parser.parse_args()

    port = 50001
    if args.port:
        port = args.port

    server = "bitcoincash.network"
    if args.server:
        server = args.server

    forward_args = map(guess_type, args.args)

    conn = Client((server, port))
    print(conn.call(args.method, *forward_args))
