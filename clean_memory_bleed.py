import os

def main():
    home = os.path.expanduser("~")
    # Telos uses single redb file now
    db_file = os.path.join(home, ".telos", "memory.redb")
    
    if os.path.exists(db_file):
        print(f"Cleaning {db_file}...")
        os.remove(db_file)
        print(f"✅ Local Agent Memory DB ({db_file}) has been hard reset.")
    else:
        print(f"File {db_file} does not exist. Skipping.")

if __name__ == "__main__":
    main()
