echo "Updating udev rules..."
sudo echo 'ATTRS{idVendor}=="1915", ATTRS{idProduct}=="c00a", MODE="660", GROUP="plugdev", TAG+="uaccess"' > /etc/udev/rules.d/99-ppk2.rules;
sudo udevadm control --reload-rules;
echo "Done! Please disconnect and reconnect your device!"
