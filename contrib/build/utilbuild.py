
def output_reader(pipe, queue):
    try:
        with pipe:
            for l in iter(pipe.readline, b''):
                queue.put(l)
    finally:
        queue.put(None)

def cargo_run(args, logging, cwd = None, faketime = None):
    import subprocess
    import shutil
    from threading import Thread
    from queue import Queue

    if faketime is None:
        faketime = []
    else:
        faketime_bin = shutil.which("faketime")
        if faketime_bin is None:
            logging.error("Please install faketime")
            raise Exception("faketime not found")
        faketime = [faketime_bin, "-m", faketime]

    cargo = shutil.which("cargo")
    args = faketime + [cargo] + args
    logging.info("Running %s", " ".join(args))
    assert cargo is not None

    p = subprocess.Popen(args,
        stdout = subprocess.PIPE, stderr = subprocess.PIPE, cwd = cwd)

    q = Queue()
    Thread(target = output_reader, args = [p.stdout, q]).start()
    Thread(target = output_reader, args = [p.stderr, q]).start()

    for line in iter(q.get, None):
        logging.debug(line.decode('utf-8').rstrip())

    p.wait()
    rc = p.returncode
    assert rc is not None
    if rc != 0:
        raise Exception("cargo failed with return code %s" % rc)
