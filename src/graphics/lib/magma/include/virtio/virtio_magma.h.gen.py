#!/usr/bin/env python3.8
# Copyright 2019 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import json
import sys

def usage():
  print('Usage:\n'
        '  virtio_magma.h.gen.py FORMAT INPUT OUTPUT\n'
        '    FORMAT   either \"fuchsia\" or \"linux\"\n'
        '    INPUT    json file containing the magma interface definition\n'
        '    OUTPUT   destination path for the virtio header file to generate\n'
        '  Example: ./virtio_magma.h.gen.py fuchsia ../magma_abi/magma.json ./virtio_magma.h\n'
        '  Generates the virtio magma header based on a provided json definition,\n'
        '  for either fuchsia or the linux kernel.')

# Generates a c or cpp style comment
def comment(lines, cpp):
  ret = [f'// {lines[0]}\n' if cpp else f'/* {lines[0]}\n']
  if cpp:
      ret.extend(f'// {line}\n' for line in lines[1:])
  else:
      ret.extend(f'   {line}\n' for line in lines[1:])
      ret.append(' */\n')
  return ''.join(ret)

# Wire formats for various widths
def wire_format_from_width(width):
  global fuchsia
  global tab
  format_fuchsia = {
    1: 'uint8_t',
    2: 'uint16_t',
    4: 'uint32_t',
    8: 'uint64_t',
  }
  format_linux = {
    1: 'u8',
    2: '__le16',
    4: '__le32',
    8: '__le64',
  }
  invalid = 'INVALID TYPE WIDTH'
  if fuchsia:
    return format_fuchsia.get(width, invalid)
  return format_linux.get(width, invalid)

# Wire format for a given type
def wire_format(type):
  # Default to 8 bytes
  width = 8
  if type.find('*') != -1: width = 8
  if type == 'uint32_t': width = 4
  if type == 'int32_t': width = 4
  if type == 'magma_bool_t': width = 1
  if type == 'magma_handle_t': width = 4
  return wire_format_from_width(width)

# License string for the top of the file.
def license():
  global fuchsia
  lines = [
    'Copyright 2018 The Fuchsia Authors. All rights reserved.',
    'Use of this source code is governed by a BSD-style license that can be',
    'found in the LICENSE file.'
  ]
  return comment(lines, fuchsia)

# Warning string about auto-generation
def warning():
  global fuchsia
  lines = [
    'NOTE: DO NOT EDIT THIS FILE! It is generated automatically by:',
    '  //src/graphics/lib/magma/include/virtio/virtio_magma.h.gen.py'
  ]
  return comment(lines, fuchsia)

# Guard macro that goes at the beginning/end of the header (after license).
def guards(begin):
  global fuchsia
  global tab
  macro = '_LINUX_VIRTIO_MAGMA_H'
  if fuchsia:
    macro = 'SRC_GRAPHICS_LIB_MAGMA_INCLUDE_VIRTIO_VIRTIO_MAGMA_H_'
  if begin:
    return '#ifndef ' + macro + '\n#define ' + macro + '\n'
  return '#endif ' + comment([macro], fuchsia)

# Includes lists.
def includes():
  if fuchsia:
    return ('#include <stdint.h>\n'
            '#include <zircon/compiler.h>\n')
  else:
    return ('#include <linux/virtio_ids.h>\n'
            '#include <linux/virtio_config.h>\n'
            '#include <linux/virtmagma.h>\n')

# Extract the non-"magma_" portion of the name of an export
def get_name(export):
  return export['name'][len('magma_'):]

# Generate a 4-digit hex string for a given integer, checking against collisions
def format_id(id, used):
  ret = '0x{:04X}'.format(id)
  if (id > len(used) or used[id]):
    raise Exception('Command ID collision: ' + ret)
  used[id] = True
  return ret

# Generate enum
def gen_enums(magma):
  global fuchsia
  global tab
  commands = tab + comment(['magma commands'], fuchsia)
  responses = tab + comment(['magma success responses'], fuchsia)
  errors = tab + comment(['magma error responses'], fuchsia)
  string_table = 'inline const char* virtio_magma_ctrl_type_string(enum virtio_magma_ctrl_type type) {\n'
  string_table += tab + 'switch (type) {\n'
  expected_response_table = 'inline enum virtio_magma_ctrl_type virtio_magma_expected_response_type(enum virtio_magma_ctrl_type type) {\n'
  expected_response_table += tab + 'switch (type) {\n'
  command_id_base = 0x1000
  response_id_base = 0x2000
  error_id_base = 0x3000
  max_id_count = 0x4000
  used = [False] * max_id_count
  for export in magma['exports']:
    name = get_name(export).upper()
    ordinal = export['ordinal']
    assert ordinal < magma['next-free-ordinal']
    command_id = command_id_base + ordinal
    response_id = response_id_base + ordinal
    commands += tab + 'VIRTIO_MAGMA_CMD_' + name + ' = ' + format_id(command_id, used) + ',\n'
    responses += tab + 'VIRTIO_MAGMA_RESP_' + name + ' = ' + format_id(response_id, used) + ',\n'
    command_id = response_id = ''
    string_table += tab + tab + 'case VIRTIO_MAGMA_CMD_' + name + ': return "VIRTIO_MAGMA_CMD_' + name + '";\n'
    string_table += tab + tab + 'case VIRTIO_MAGMA_RESP_' + name + ': return "VIRTIO_MAGMA_RESP_' + name + '";\n'
    expected_response_table += tab + tab + 'case VIRTIO_MAGMA_CMD_' + name + ': return VIRTIO_MAGMA_RESP_' + name + ';\n'
  error_names = [
    'VIRTIO_MAGMA_RESP_ERR_UNIMPLEMENTED',
    'VIRTIO_MAGMA_RESP_ERR_INTERNAL',
    'VIRTIO_MAGMA_RESP_ERR_HOST_DISCONNECTED',
    'VIRTIO_MAGMA_RESP_ERR_OUT_OF_MEMORY',
    'VIRTIO_MAGMA_RESP_ERR_INVALID_COMMAND',
    'VIRTIO_MAGMA_RESP_ERR_INVALID_ARGUMENT'
  ]
  error_id = error_id_base + 1
  for error_name in error_names:
    errors += tab + error_name + ' = ' + format_id(error_id, used) + ',\n'
    string_table += tab + tab + 'case ' + error_name + ': return "' + error_name + '";\n'
    error_id = error_id + 1
  string_table += tab + tab + 'default: return "[invalid virtio_magma_ctrl_type]";\n'
  string_table += tab + '}\n'
  string_table += '}\n'
  expected_response_table += tab + tab + 'default: return VIRTIO_MAGMA_RESP_ERR_INVALID_COMMAND;\n'
  expected_response_table += tab + '}\n'
  expected_response_table += '}\n'
  ret = 'enum virtio_magma_ctrl_type {\n'
  ret += commands
  ret += responses
  ret += errors
  if fuchsia:
    ret += '} __PACKED;\n\n'
  else:
    ret += '} __attribute((packed));\n\n'
  ret += string_table + '\n'
  ret += expected_response_table
  return ret

# Format command or response struct for an export
def format_struct(export, ctrl):
  global fuchsia
  global tab
  name = 'virtio_magma_' + get_name(export) + '_' + ('ctrl' if ctrl else 'resp')
  ret = ''
  if fuchsia:
    ret += 'typedef '
  ret += 'struct ' + name + ' {\n'
  if fuchsia:
    ret += tab + 'virtio_magma_ctrl_hdr_t hdr;\n'
  else:
    ret += tab + 'struct virtio_magma_ctrl_hdr hdr;\n'
  for argument in export['arguments']:
    # Include this argument iff out and resp or !out and ctrl
    use = False
    if argument['name'].find('_out') == -1:
      if ctrl:
        use = True
    else:
      if not ctrl:
        use = True
    if use:
      ret += tab + wire_format(argument['type']) + ' ' + argument['name'] + ';\n'
  # Add return value, if any
  if not ctrl:
    if export['type'] != 'void':
      ret += tab + wire_format(export['type']) + ' result_return;\n'
  if fuchsia:
    ret += '} __PACKED ' + name + '_t;\n'
  else:
    ret += '} __attribute((packed));\n'
  return ret

def config_type():
  global fuchsia
  global tab
  ret = ''
  if fuchsia:
    ret += 'typedef '
  ret += 'struct virtio_magma_config {\n'
  ret += tab + wire_format('uint8_t') + ' dummy;\n'
  if fuchsia:
    ret += '} __PACKED virtio_magma_config_t;\n'
  else:
    ret += '} __attribute((packed));\n'
  return ret

# Common control header struct
def ctrl_hdr():
  global fuchsia
  global tab
  ret = ''
  if fuchsia:
    ret += 'typedef '
  ret += 'struct virtio_magma_ctrl_hdr {\n'
  ret += tab + wire_format('uint32_t') + ' type;\n'
  ret += tab + wire_format('uint32_t') + ' flags;\n'
  if fuchsia:
    ret += '} __PACKED virtio_magma_ctrl_hdr_t;\n'
  else:
    ret += '} __attribute((packed));\n'
  return ret

fuchsia = True
tab = '    '
def main():
  global fuchsia
  global tab
  if (len(sys.argv) != 4):
    usage()
    exit(-1)
  if (sys.argv[1] == 'linux'):
    fuchsia = False
    tab = '\t'
  elif (sys.argv[1] != 'fuchsia'):
    usage()
    exit(-2)
  with open(sys.argv[2], 'r') as file:
    with open(sys.argv[3], 'w') as dest:
      magma = json.load(file)['magma-interface']
      header = license() + '\n'
      header += warning() + '\n'
      header += guards(True) + '\n'
      header += includes() + '\n'
      if fuchsia:
        header += '__BEGIN_CDECLS\n\n'
      header += config_type() + '\n'
      header += gen_enums(magma) + '\n'
      header += ctrl_hdr() + '\n'
      for export in magma['exports']:
        header += format_struct(export, True) + '\n'
        header += format_struct(export, False) + '\n'
      if fuchsia:
        header += '__END_CDECLS\n\n'
      header += guards(False)
      dest.write(header)

if __name__ == '__main__':
  sys.exit(main())
