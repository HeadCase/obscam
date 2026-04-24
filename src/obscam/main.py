import zwoasi as asi

SDK_PATH = "/Users/gheadley/nas/develop/python/obscam/libASICamera/libASICamera2.dylib"

asi.init(SDK_PATH)


def main():
    print(asi.get_num_cameras())


if __name__ == "__main__":
    main()
