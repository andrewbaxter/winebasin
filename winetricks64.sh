# d3dx library shims are provided by wine, the winetricks versions are the full windows versions.
# The wine-provided versions are expected to be sufficient in the long term.
#
# dxvk/vkd3d are implementations for the shim libraries (?) - shims are separate.
#
# Major versions of all libraries are needed, not minor versions (with some exceptions).
# 
# Higher versions first, in case the higher version obviates the older version (don't have to recreate).
winetricks -q \
	vcrun2022 \
	vcrun2013 \
	vcrun2012 \
	vcrun2010 \
	vcrun2008 \
	vcrun2005 \
	vcrun2003 \
	vcrun6sp6 \
	vcrun6 \
	allfonts \
	allcodecs \
	dotnet7 \
	dotnet6 \
	dotnet48 \
	dotnet472 \
	dotnet471 \
	dotnet40 \
	dotnet35sp1 \
	dotnetcore3 \
	dotnetcore2 \
	dxvk2030 \
	vkd3d \
	;
