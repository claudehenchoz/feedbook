#!/bin/sh

# 1. Show a toast that it started (3000 = 3 seconds)
qndb -m mwcToast 3000 "Feedbook is downloading articles..."

# 2. Run your Rust app
/mnt/onboard/.adds/feedbook/feedbook

# 3. Check if successful
if [ $? -eq 0 ]; then
    # Make sure the file is actually written to the disk before scanning
    sync 
    
    # Show your success message! 
    qndb -m mwcToast 3000 "Feedbook completed successfully!"
    
    # Safely trigger the native "Importing content..." screen.
    # The -t 30000 and -s wait for the import sequence to finish before closing the script.
    qndb -t 30000 -s pfmDoneProcessing -m pfmRescanBooksFull
else
    # Show an error message if the Rust app panicked or failed
    qndb -m mwcToast 4000 "Feedbook failed to update."
fi
