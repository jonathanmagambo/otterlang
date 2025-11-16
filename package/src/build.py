import os
from pathlib import Path
this_file = Path(os.path.abspath(__file__))
otter_deb_path = (this_file.parent.parent / 'otterlang' / 'DEBIAN').resolve()
print(f'Will create at {otter_deb_path}')
if os.path.exists(otter_deb_path):
    clear = input('An output appears to already exist. Clear it? [y/N]: ')
    if clear.lower() == 'y':
        os.system(f'rm -r {otter_deb_path}')

if not os.path.exists(otter_deb_path):
    if not os.path.exists(otter_deb_path.parent.resolve()):
        os.mkdir(otter_deb_path.parent.resolve())
    os.mkdir(otter_deb_path)

otter_bin_path = (this_file.parent.parent.parent / 'target' / 'release' / 'otter').resolve()
if not os.path.exists(otter_bin_path):
    print('First you need to build the binaries')
    exit()

if not os.path.exists((otter_deb_path / 'bin').resolve()):
    os.mkdir((otter_deb_path / 'bin').resolve())

os.system(f'cp {otter_bin_path} {(otter_deb_path / 'bin').resolve()}')
os.system(f'cp {(this_file.parent.parent / 'control').resolve()} {otter_deb_path}')
os.system(f'cp {(this_file.parent.parent / 'changelog').resolve()} {otter_deb_path}')
os.system(f'cp {(this_file.parent.parent / 'copyright').resolve()} {otter_deb_path}')
os.system(f'cp {(this_file.parent.parent / 'rules').resolve()} {otter_deb_path}')

#original_dir = os.getcwd()
#os.chdir(otter_deb_path.resolve())
#os.system('pwd')
os.system(f'dpkg-deb -b {otter_deb_path.parent} {(this_file.parent.parent / 'otterlang.deb')}')
#os.chdir(original_dir)